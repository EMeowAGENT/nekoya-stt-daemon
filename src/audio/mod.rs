//! 音频采集与分块（PCM16 / 16 kHz / mono）。
//!
//! - 纯逻辑：分块、预滚环形缓冲（可离线单测）。
//! - 适配器：`FileSource`（回放 pcm 夹具）、`PwRecordSource`（PipeWire 采集子进程）。
//!
//! 采集实现与识别解耦：识别端只从 `tokio::sync::mpsc` 收块。

use std::collections::VecDeque;
use std::io::Read;

pub const SAMPLE_RATE: u32 = 16_000;
pub const BYTES_PER_SAMPLE: usize = 2; // s16le
pub const CHUNK_MS: u64 = 100;

/// 100ms 对应的字节数（16k * 2B * 0.1 = 3200）。
pub fn chunk_size_for_ms(ms: u64) -> usize {
    SAMPLE_RATE as usize * BYTES_PER_SAMPLE * ms as usize / 1000
}

pub const CHUNK_BYTES: usize = SAMPLE_RATE as usize * BYTES_PER_SAMPLE * CHUNK_MS as usize / 1000;

/// 按 `size` 切分（最后一段可能不足）。
pub fn chunks(data: &[u8], size: usize) -> impl Iterator<Item = &[u8]> {
    let size = size.max(1);
    data.chunks(size)
}

/// 预滚环形缓冲：保留最近 `ms` 毫秒音频，用于 PTT 起手补发，避免吞首字。
#[derive(Debug)]
pub struct Preroll {
    buf: VecDeque<u8>,
    cap: usize,
}

impl Preroll {
    pub fn new(ms: u64) -> Self {
        Self {
            buf: VecDeque::new(),
            cap: chunk_size_for_ms(ms),
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        let cap = self.cap.max(1);
        for &b in data {
            if self.buf.len() == cap {
                self.buf.pop_front();
            }
            self.buf.push_back(b);
        }
    }

    /// 取出全部并清空。
    pub fn take(&mut self) -> Vec<u8> {
        self.buf.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// 同步音频源（在阻塞线程里泵入识别通道）。
pub trait AudioSource: Send {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize>;
}

/// 从文件/任意 Reader 读取（测试夹具回放）。
pub struct FileSource(pub std::fs::File);

impl AudioSource for FileSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(out)
    }
}

impl FileSource {
    pub fn open(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        Ok(Self(std::fs::File::open(path)?))
    }
}

/// PipeWire 采集子进程（`pw-record`）。Drop 时终止。
pub struct PwRecordSource {
    child: std::process::Child,
}

impl PwRecordSource {
    /// `source` 为可选的 PipeWire 输入源名（None = 系统默认）。
    pub fn spawn(source: Option<&str>) -> anyhow::Result<Self> {
        let mut cmd = std::process::Command::new("pw-record");
        cmd.args(["--rate", "16000", "--channels", "1", "--format", "s16"]);
        if let Some(s) = source {
            cmd.args(["--target", s]);
        }
        cmd.arg("-"); // stdout
        cmd.stdout(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("无法启动 pw-record: {e}"))?;
        Ok(Self { child })
    }
}

impl AudioSource for PwRecordSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self.child.stdout.as_mut() {
            Some(stdout) => stdout.read(out),
            None => Ok(0),
        }
    }
}

impl Drop for PwRecordSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_matches_spec() {
        assert_eq!(chunk_size_for_ms(100), 3200);
        assert_eq!(CHUNK_BYTES, 3200);
    }

    #[test]
    fn chunking_splits_and_keeps_tail() {
        let data = vec![0u8; 3200 * 2 + 100];
        let got: Vec<usize> = chunks(&data, 3200).map(|c| c.len()).collect();
        assert_eq!(got, vec![3200, 3200, 100]);
    }

    #[test]
    fn preroll_keeps_last_window() {
        let mut p = Preroll::new(1); // 1ms = 32 字节
        assert_eq!(p.len(), 0);
        p.push(&[1u8; 40]); // 超过容量，只保留最后 32
        assert_eq!(p.len(), 32);
        let out = p.take();
        assert_eq!(out.len(), 32);
        assert!(p.is_empty());
    }

    #[test]
    fn preroll_take_clears() {
        let mut p = Preroll::new(100);
        p.push(&[7u8; 10]);
        assert_eq!(p.take().len(), 10);
        assert!(p.take().is_empty());
    }
}
