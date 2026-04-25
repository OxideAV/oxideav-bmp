//! BMP container: one single-image file becomes one [`Packet`] on
//! stream `0`. Matches how other single-image codecs in the workspace
//! (`oxideav-png` for non-APNG, `oxideav-webp` for static WEBP) plug
//! into the container pipeline.

use std::io::{Read, SeekFrom, Write};

use oxideav_core::{
    CodecId, CodecParameters, CodecResolver, Error, MediaType, Packet, PixelFormat, Result,
    StreamInfo, TimeBase,
};
use oxideav_core::{
    ContainerRegistry, Demuxer, Muxer, ProbeData, ProbeScore, ReadSeek, WriteSeek, MAX_PROBE_SCORE,
};

use crate::types::{read_u16_le, read_u32_le, BMP_MAGIC};

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("bmp", open_demuxer);
    reg.register_muxer("bmp", open_muxer);
    reg.register_extension("bmp", "bmp");
    reg.register_extension("dib", "bmp");
    reg.register_probe("bmp", probe);
}

fn probe(data: &ProbeData) -> ProbeScore {
    if data.buf.len() >= 2 && read_u16_le(data.buf, 0) == BMP_MAGIC {
        MAX_PROBE_SCORE
    } else if matches!(data.ext, Some("bmp") | Some("dib")) {
        oxideav_core::PROBE_SCORE_EXTENSION
    } else {
        0
    }
}

pub fn open_demuxer(
    mut input: Box<dyn ReadSeek>,
    _codecs: &dyn CodecResolver,
) -> Result<Box<dyn Demuxer>> {
    // Slurp the whole file; BMPs are tiny by modern standards.
    input.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    if buf.len() < 26 || read_u16_le(&buf, 0) != BMP_MAGIC {
        return Err(Error::invalid("BMP: missing 'BM' signature"));
    }
    // Pull width / height out of the DIB header so the returned
    // StreamInfo carries accurate metadata — some callers (e.g. the
    // job graph) read these before they see any frames.
    let width = read_u32_le(&buf, 18);
    let height_signed = i32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]);
    let height = height_signed.unsigned_abs();

    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(width);
    params.height = Some(height);
    params.pixel_format = Some(PixelFormat::Rgba);
    let stream = StreamInfo {
        index: 0,
        params,
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    };
    Ok(Box::new(BmpDemuxer {
        streams: vec![stream],
        data: Some(buf),
    }))
}

struct BmpDemuxer {
    streams: Vec<StreamInfo>,
    /// `None` once the sole packet has been emitted.
    data: Option<Vec<u8>>,
}

impl Demuxer for BmpDemuxer {
    fn format_name(&self) -> &str {
        "bmp"
    }
    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }
    fn next_packet(&mut self) -> Result<Packet> {
        match self.data.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.pts = Some(0);
                pkt.dts = Some(0);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => Err(Error::Eof),
        }
    }
}

pub fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        return Err(Error::invalid(
            "BMP muxer: expected exactly one video stream",
        ));
    }
    if streams[0].params.media_type != MediaType::Video {
        return Err(Error::invalid("BMP muxer: stream must be video"));
    }
    Ok(Box::new(BmpMuxer { output }))
}

struct BmpMuxer {
    output: Box<dyn WriteSeek>,
}

impl Muxer for BmpMuxer {
    fn format_name(&self) -> &str {
        "bmp"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        self.output.write_all(&packet.data)?;
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}
