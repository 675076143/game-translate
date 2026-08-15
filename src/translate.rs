use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::{logger, terminology};

const ENDPOINT: &str = "http://127.0.0.1:11434/api/generate";
const MODEL: &str = "qwen3:4b-instruct";
const CACHE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy)]
pub enum TranslationSource {
    Template,
    MemoryCache,
    FileCache,
    Model,
}

impl TranslationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Template => "template",
            Self::MemoryCache => "memory-cache",
            Self::FileCache => "file-cache",
            Self::Model => "model",
        }
    }
}

pub struct TranslationJob {
    pub id: u64,
    pub original: String,
}

pub struct Translation {
    pub id: u64,
    pub original: String,
    pub translated: String,
    pub source: TranslationSource,
    pub elapsed: Duration,
}

pub struct TranslationOutcome {
    pub id: u64,
    pub result: Result<Translation>,
}

pub struct Translator {
    client: Client,
    cache: HashMap<String, String>,
    cache_order: VecDeque<String>,
    file_cache: HashMap<String, String>,
    cache_path: PathBuf,
}

impl Translator {
    pub fn new(cache_path: PathBuf) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .build()
            .context("无法创建 HTTP 客户端")?;
        let file_cache = load_file_cache(&cache_path)?;
        logger::write(
            "translate-cache",
            &format!("loaded_entries={}", file_cache.len()),
        );
        Ok(Self {
            client,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            file_cache,
            cache_path,
        })
    }

    pub fn warm_up(&self) {
        let started = Instant::now();
        let result = self.generate("");
        logger::write(
            "translate-warmup",
            &format!(
                "elapsed_ms={} success={}",
                started.elapsed().as_millis(),
                result.is_ok()
            ),
        );
    }

    pub fn translate(&mut self, job: TranslationJob) -> Result<Translation> {
        let started = Instant::now();
        let (translated, source) =
            if let Some(translated) = terminology::translate_battle(&job.original) {
                (translated, TranslationSource::Template)
            } else if let Some(translated) = self.cache.get(&job.original) {
                (translated.clone(), TranslationSource::MemoryCache)
            } else if let Some(translated) = self.file_cache.get(&job.original).cloned() {
                self.insert_memory(job.original.clone(), translated.clone());
                (translated, TranslationSource::FileCache)
            } else {
                let translated = self.translate_model(&job.original)?;
                self.insert_file_cache(job.original.clone(), translated.clone())?;
                (translated, TranslationSource::Model)
            };
        Ok(Translation {
            id: job.id,
            original: job.original,
            translated,
            source,
            elapsed: started.elapsed(),
        })
    }

    fn translate_model(&self, original: &str) -> Result<String> {
        let prompt = format!(
            "英译简中，只输出译文，使用宝可梦官方术语，纠正明显OCR错字。\
术语：Pokémon=宝可梦，Pecha Berry=桃桃果，Berries Pocket=树果口袋，\
Potion=伤药，Medicine Pocket=药品口袋，Pokémon Center=宝可梦中心，\
Poké Mart=友好商店。\n{original}"
        );
        let translated = self.generate(&prompt)?;
        if translated.trim().is_empty() {
            bail!("本地翻译模型返回空文本");
        }
        Ok(translated.trim().to_owned())
    }

    fn generate(&self, prompt: &str) -> Result<String> {
        let response = self
            .client
            .post(ENDPOINT)
            .json(&json!({
                "model": MODEL,
                "prompt": prompt,
                "stream": false,
                "think": false,
                "keep_alive": "30m",
                "options": {
                    "temperature": 0,
                    "num_ctx": 2048,
                    "num_predict": 100
                }
            }))
            .send()
            .context("无法连接本地 Ollama 翻译服务")?;
        let status = response.status();
        let body = response.text().context("无法读取 Ollama 响应")?;
        if !status.is_success() {
            bail!("Ollama 返回 HTTP {status}: {}", body.trim());
        }
        let payload: Value = serde_json::from_str(&body).context("无法解析 Ollama 响应")?;
        payload
            .get("response")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("Ollama 响应缺少译文")
    }

    fn insert_memory(&mut self, original: String, translated: String) {
        if self.cache.contains_key(&original) {
            return;
        }
        if self.cache.len() == CACHE_CAPACITY
            && let Some(oldest) = self.cache_order.pop_front()
        {
            self.cache.remove(&oldest);
        }
        self.cache_order.push_back(original.clone());
        self.cache.insert(original, translated);
    }

    fn insert_file_cache(&mut self, original: String, translated: String) -> Result<()> {
        self.insert_memory(original.clone(), translated.clone());
        if self.file_cache.contains_key(&original) {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cache_path)
            .context("无法打开翻译文件缓存")?;
        serde_json::to_writer(&mut file, &[&original, &translated])?;
        writeln!(file)?;
        self.file_cache.insert(original, translated);
        Ok(())
    }
}

fn load_file_cache(path: &Path) -> Result<HashMap<String, String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error).context("无法读取翻译文件缓存"),
    };
    let mut cache = HashMap::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let [original, translated]: [String; 2] =
            serde_json::from_str(&line).context("翻译文件缓存损坏")?;
        cache.insert(original, translated);
    }
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{CACHE_CAPACITY, Translator};

    #[test]
    fn cache_is_bounded() {
        let mut translator =
            Translator::new(PathBuf::from("/tmp/nonexistent-game-translate-cache")).unwrap();
        for index in 0..=CACHE_CAPACITY {
            translator.insert_memory(index.to_string(), "translated".into());
        }
        assert_eq!(translator.cache.len(), CACHE_CAPACITY);
        assert!(!translator.cache.contains_key("0"));
    }

    #[test]
    fn file_cache_survives_a_new_translator() {
        let path = std::env::temp_dir().join(format!(
            "game-translate-cache-test-{}.jsonl",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut first = Translator::new(path.clone()).unwrap();
        first
            .insert_file_cache("Hello".into(), "你好".into())
            .unwrap();
        let second = Translator::new(path.clone()).unwrap();
        assert_eq!(second.file_cache.get("Hello").unwrap(), "你好");
        fs::remove_file(path).unwrap();
    }
}
