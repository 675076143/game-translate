use std::str::FromStr;

use anyhow::{Context, Result, bail};
use image::DynamicImage;
use libwayshot::{
    LogicalRegion, WayshotConnection,
    region::{Position, Region, Size},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl FromStr for Geometry {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let (position, size) = value
            .split_once(' ')
            .context("选区格式应为 X,Y WIDTHxHEIGHT")?;
        let (x, y) = position.split_once(',').context("选区坐标无效")?;
        let (width, height) = size.split_once('x').context("选区尺寸无效")?;
        let geometry = Self {
            x: x.parse().context("选区 X 坐标无效")?,
            y: y.parse().context("选区 Y 坐标无效")?,
            width: width.parse().context("选区宽度无效")?,
            height: height.parse().context("选区高度无效")?,
        };
        if geometry.width == 0 || geometry.height == 0 {
            bail!("字幕区域不能为空");
        }
        Ok(geometry)
    }
}

pub struct Capture {
    connection: WayshotConnection,
    region: LogicalRegion,
}

impl Capture {
    pub fn new(geometry: Geometry) -> Result<Self> {
        let connection = WayshotConnection::new().context("无法连接 Wayland screencopy 协议")?;
        let region = logical_region(geometry);
        Ok(Self { connection, region })
    }

    pub fn frame(&mut self) -> Result<DynamicImage> {
        self.connection
            .screenshot(self.region, false)
            .context("screencopy 捕获失败")
    }

    pub fn set_geometry(&mut self, geometry: Geometry) {
        self.region = logical_region(geometry);
    }
}

fn logical_region(geometry: Geometry) -> LogicalRegion {
    LogicalRegion {
        inner: Region {
            position: Position {
                x: geometry.x,
                y: geometry.y,
            },
            size: Size {
                width: geometry.width,
                height: geometry.height,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::Geometry;

    #[test]
    fn parses_slurp_geometry() {
        let geometry: Geometry = "739,910 1543x350".parse().unwrap();
        assert_eq!(geometry.x, 739);
        assert_eq!(geometry.y, 910);
        assert_eq!(geometry.width, 1543);
        assert_eq!(geometry.height, 350);
    }
}
