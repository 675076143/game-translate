use std::{
    env,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::capture::Geometry;

#[derive(Debug)]
struct Window {
    address: String,
    title: String,
    geometry: Geometry,
    workspace_id: i64,
}

pub struct WindowTracker {
    socket: PathBuf,
    address: String,
    title: String,
    offset_x: i32,
    offset_y: i32,
    width: u32,
    height: u32,
}

impl WindowTracker {
    pub fn bind(selection: Geometry) -> Result<Self> {
        let socket = socket_path()?;
        let windows = query_windows(&socket)?;
        let window = windows
            .into_iter()
            .filter(|window| window.title != "GT-Translate")
            .max_by_key(|window| intersection_area(selection, window.geometry))
            .filter(|window| intersection_area(selection, window.geometry) > 0)
            .context("选区不在任何 Hyprland 窗口内")?;
        Ok(Self::from_window(socket, window, selection))
    }

    pub fn select() -> Result<Self> {
        let socket = socket_path()?;
        let active_workspaces = query_active_workspaces(&socket)?;
        let windows: Vec<_> = query_windows(&socket)?
            .into_iter()
            .filter(|window| {
                window.title != "GT-Translate" && active_workspaces.contains(&window.workspace_id)
            })
            .collect();
        let mut child = Command::new("slurp")
            .args(["-r", "-f", "%l"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("无法启动 slurp 窗口选择")?;
        {
            let input = child.stdin.as_mut().context("无法写入窗口列表")?;
            for window in &windows {
                writeln!(
                    input,
                    "{},{} {}x{} {} {}",
                    window.geometry.x,
                    window.geometry.y,
                    window.geometry.width,
                    window.geometry.height,
                    window.address,
                    window.title.replace('\n', " ")
                )?;
            }
        }
        let output = child.wait_with_output().context("等待窗口选择失败")?;
        if !output.status.success() {
            bail!("已取消程序窗口选择");
        }
        let label = String::from_utf8(output.stdout).context("slurp 返回了无效文本")?;
        let address = label
            .split_whitespace()
            .next()
            .context("窗口选择结果为空")?;
        let window = windows
            .into_iter()
            .find(|window| window.address == address)
            .context("选中的窗口已不存在")?;
        let selection = window.geometry;
        Ok(Self::from_window(socket, window, selection))
    }

    fn from_window(socket: PathBuf, window: Window, selection: Geometry) -> Self {
        Self {
            socket,
            address: window.address,
            title: window.title,
            offset_x: selection.x - window.geometry.x,
            offset_y: selection.y - window.geometry.y,
            width: selection.width,
            height: selection.height,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn geometry(&self) -> Result<Option<Geometry>> {
        let active_workspaces = query_active_workspaces(&self.socket)?;
        let window = query_windows(&self.socket)?
            .into_iter()
            .find(|window| window.address == self.address);
        Ok(window
            .filter(|window| active_workspaces.contains(&window.workspace_id))
            .map(|window| Geometry {
                x: window.geometry.x + self.offset_x,
                y: window.geometry.y + self.offset_y,
                width: self.width,
                height: self.height,
            }))
    }
}

fn socket_path() -> Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let signature =
        env::var_os("HYPRLAND_INSTANCE_SIGNATURE").context("not running under Hyprland")?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock"))
}

fn query_windows(socket: &PathBuf) -> Result<Vec<Window>> {
    let values = query(socket, b"j/clients")?;
    values
        .as_array()
        .context("Hyprland clients response is not an array")?
        .iter()
        .map(parse_window)
        .collect()
}

fn query_active_workspaces(socket: &PathBuf) -> Result<Vec<i64>> {
    let values = query(socket, b"j/monitors")?;
    values
        .as_array()
        .context("Hyprland monitors response is not an array")?
        .iter()
        .map(|monitor| {
            monitor["activeWorkspace"]["id"]
                .as_i64()
                .context("monitor has no active workspace")
        })
        .collect()
}

fn query(socket: &PathBuf, command: &[u8]) -> Result<Value> {
    let mut stream = UnixStream::connect(socket).context("cannot connect to Hyprland IPC")?;
    stream
        .write_all(command)
        .context("cannot query Hyprland IPC")?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("cannot read Hyprland IPC")?;
    serde_json::from_str(&response).context("invalid Hyprland IPC JSON")
}

fn parse_window(value: &Value) -> Result<Window> {
    let pair = |name: &str| -> Result<(i32, i32)> {
        let values = value[name]
            .as_array()
            .with_context(|| format!("missing {name}"))?;
        if values.len() != 2 {
            bail!("invalid {name}");
        }
        Ok((
            values[0].as_i64().context("invalid coordinate")? as i32,
            values[1].as_i64().context("invalid coordinate")? as i32,
        ))
    };
    let (x, y) = pair("at")?;
    let (width, height) = pair("size")?;
    Ok(Window {
        address: value["address"]
            .as_str()
            .context("missing address")?
            .to_owned(),
        title: value["title"].as_str().unwrap_or_default().to_owned(),
        geometry: Geometry {
            x,
            y,
            width: width.try_into().context("negative width")?,
            height: height.try_into().context("negative height")?,
        },
        workspace_id: value["workspace"]["id"]
            .as_i64()
            .context("missing workspace id")?,
    })
}

fn intersection_area(left: Geometry, right: Geometry) -> u64 {
    let x = i64::from(left.x).max(i64::from(right.x));
    let y = i64::from(left.y).max(i64::from(right.y));
    let right_edge = (i64::from(left.x) + i64::from(left.width))
        .min(i64::from(right.x) + i64::from(right.width));
    let bottom = (i64::from(left.y) + i64::from(left.height))
        .min(i64::from(right.y) + i64::from(right.height));
    (right_edge - x).max(0) as u64 * (bottom - y).max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::intersection_area;
    use crate::capture::Geometry;

    #[test]
    fn calculates_window_overlap() {
        let window = Geometry {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        let selection = Geometry {
            x: 200,
            y: 500,
            width: 300,
            height: 200,
        };
        assert_eq!(intersection_area(window, selection), 60_000);
    }
}
