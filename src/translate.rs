use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::Value;

use crate::terminology;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";

pub struct Translation {
    pub original: String,
    pub translated: String,
}

pub struct Translator {
    client: Client,
}

impl Translator {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .build()
            .context("无法创建 HTTP 客户端")?;
        Ok(Self { client })
    }

    pub fn translate(&self, original: &str) -> Result<String> {
        if let Some(translated) = terminology::translate_battle(original) {
            return Ok(translated);
        }
        let payload: Value = self
            .client
            .get(ENDPOINT)
            .query(&[
                ("client", "gtx"),
                ("sl", "en"),
                ("tl", "zh-CN"),
                ("dt", "t"),
                ("q", original),
            ])
            .send()
            .context("翻译请求失败")?
            .error_for_status()
            .context("翻译服务返回错误状态")?
            .json()
            .context("无法解析翻译响应")?;
        let segments = payload
            .get(0)
            .and_then(Value::as_array)
            .context("翻译响应缺少文本")?;
        let translated = segments
            .iter()
            .filter_map(|segment| segment.get(0).and_then(Value::as_str))
            .collect::<String>();
        if translated.trim().is_empty() {
            bail!("翻译服务返回空文本");
        }
        Ok(translated)
    }
}
