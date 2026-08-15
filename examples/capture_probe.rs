use std::{env, time::Instant};

use anyhow::{Context, Result};
use libwayshot::{
    LogicalRegion, WayshotConnection,
    region::{Position, Region, Size},
};

fn main() -> Result<()> {
    let value = env::args()
        .nth(1)
        .context("usage: capture_probe 'X,Y WIDTHxHEIGHT' [frame-count]")?;
    let frame_count: u32 = env::args()
        .nth(2)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(50);
    let (position, size) = value.split_once(' ').context("invalid geometry")?;
    let (x, y) = position.split_once(',').context("invalid position")?;
    let (width, height) = size.split_once('x').context("invalid size")?;
    let region = LogicalRegion {
        inner: Region {
            position: Position {
                x: x.parse()?,
                y: y.parse()?,
            },
            size: Size {
                width: width.parse()?,
                height: height.parse()?,
            },
        },
    };
    let connection = WayshotConnection::new()?;
    let started = Instant::now();
    let mut pixels = 0_u64;
    for _ in 0..frame_count {
        let image = connection.screenshot(region, false)?;
        pixels += u64::from(image.width()) * u64::from(image.height());
    }
    let elapsed = started.elapsed();
    println!(
        "{frame_count} frames in {:.3}s ({:.1} fps), {} pixels",
        elapsed.as_secs_f64(),
        f64::from(frame_count) / elapsed.as_secs_f64(),
        pixels
    );
    Ok(())
}
