//! Incremental SSE framing. HTTP chunks are not event or UTF-8 boundaries.
use super::RelayError;

#[derive(Debug)]
pub(crate) struct Frame {
    pub event: Option<String>,
    pub data: String,
}

pub(crate) struct Parser {
    line: Vec<u8>,
    data: String,
    event: Option<String>,
    has_data: bool,
    skip_lf: bool,
    first_line: bool,
    bytes: usize,
    limit: usize,
}
impl Parser {
    pub fn new(limit: usize) -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event: None,
            has_data: false,
            skip_lf: false,
            first_line: true,
            bytes: 0,
            limit,
        }
    }
    /// Process one byte so callers can apply backpressure between events even
    /// when a single network chunk contains thousands of small events.
    pub fn feed(&mut self, byte: u8) -> Result<Option<Frame>, RelayError> {
        let skip_lf = self.skip_lf && byte == b'\n';
        self.skip_lf = false;
        // A CR already dispatched a blank line when bytes is zero. Its LF
        // belongs to that completed boundary, not the next event. Otherwise
        // CRLF's LF still consumes a byte of the current event's budget.
        if skip_lf && self.bytes == 0 {
            return Ok(None);
        }
        self.bytes = self
            .bytes
            .checked_add(1)
            .ok_or(RelayError::Limit("SSE event"))?;
        if self.bytes > self.limit {
            return Err(RelayError::Limit("SSE event"));
        }
        if skip_lf {
            return Ok(None);
        }
        match byte {
            b'\r' => {
                self.skip_lf = true;
                self.end_line()
            }
            b'\n' => self.end_line(),
            _ => {
                self.line.push(byte);
                Ok(None)
            }
        }
    }
    fn end_line(&mut self) -> Result<Option<Frame>, RelayError> {
        let raw = std::mem::take(&mut self.line);
        let line =
            std::str::from_utf8(&raw).map_err(|_| RelayError::Protocol("SSE is not UTF-8"))?;
        let line = if self.first_line {
            self.first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            self.bytes = 0;
            let event = self.event.take();
            if self.has_data {
                self.has_data = false;
                return Ok(Some(Frame {
                    event,
                    data: std::mem::take(&mut self.data),
                }));
            }
            self.data.clear();
            return Ok(None);
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // id/retry do not authorize reconnecting or replaying a paid POST.
            _ => {}
        }
        Ok(None)
    }
    pub fn finish(&self) -> Result<(), RelayError> {
        if !self.line.is_empty() || self.has_data {
            Err(RelayError::Truncated)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_crlf_multiline_comments_and_bom() {
        let raw = "\u{feff}: hello\r\nevent: delta\r\ndata: {\"text\":\"鲸鱼\",\r\ndata: \"n\":1}\r\n\r\n";
        let mut parser = Parser::new(1024);
        let mut out = Vec::new();
        for b in raw.as_bytes() {
            if let Some(f) = parser.feed(*b).unwrap() {
                out.push(f);
            }
        }
        parser.finish().unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event.as_deref(), Some("delta"));
        assert_eq!(out[0].data, "{\"text\":\"鲸鱼\",\n\"n\":1}");
    }
    #[test]
    fn lone_cr_and_empty_data_are_valid() {
        let mut p = Parser::new(128);
        let mut out = Vec::new();
        for b in b"data:\r\rdata: [DONE]\r\r" {
            if let Some(f) = p.feed(*b).unwrap() {
                out.push(f.data);
            }
        }
        assert_eq!(out, ["", "[DONE]"]);
        p.finish().unwrap();
    }
    #[test]
    fn unterminated_and_invalid_utf8_are_rejected() {
        let mut p = Parser::new(128);
        for b in b"data: partial" {
            p.feed(*b).unwrap();
        }
        assert_eq!(p.finish(), Err(RelayError::Truncated));
        let mut p = Parser::new(128);
        p.feed(255).unwrap();
        assert!(matches!(p.feed(b'\n'), Err(RelayError::Protocol(_))));
    }
    #[test]
    fn comments_and_unknown_fields_cannot_evade_the_event_bound() {
        let mut p = Parser::new(16);
        for b in b":1234567\n:123456" {
            p.feed(*b).unwrap();
        }
        assert!(matches!(p.feed(b'7'), Err(RelayError::Limit(_))));
    }
    #[test]
    fn crlf_inside_an_event_counts_toward_the_byte_limit() {
        // The second CR closes the frame; its optional LF is framing only.
        // All bytes before that boundary must count, including CRLF's LF.
        let raw = b"data: x\r\ndata: y\r\r";
        let mut p = Parser::new(raw.len() - 1);
        assert!(matches!(
            raw.iter().try_for_each(|&byte| p.feed(byte).map(|_| ())),
            Err(RelayError::Limit("SSE event"))
        ));
        let mut p = Parser::new(raw.len());
        let mut frames = Vec::new();
        for &byte in raw {
            if let Some(frame) = p.feed(byte).unwrap() {
                frames.push(frame.data);
            }
        }
        assert_eq!(frames, ["x\ny"]);
        p.finish().unwrap();
    }
    #[test]
    fn crlf_between_events_does_not_consume_the_next_event_limit() {
        let mut p = Parser::new(b"data: x\r\r".len());
        let mut frames = Vec::new();
        for &byte in b"data: x\r\r\ndata: y\r\r\n" {
            if let Some(frame) = p.feed(byte).unwrap() {
                frames.push(frame.data);
            }
        }
        assert_eq!(frames, ["x", "y"]);
        assert_eq!(p.bytes, 0);
        p.finish().unwrap();
    }
    #[test]
    fn many_events_do_not_accumulate() {
        let mut p = Parser::new(32);
        let mut count = 0;
        for _ in 0..10_000 {
            for b in b"data: x\n\n" {
                if p.feed(*b).unwrap().is_some() {
                    count += 1;
                }
            }
        }
        assert_eq!(count, 10_000);
        assert_eq!(p.bytes, 0);
        assert!(p.data.is_empty());
    }
}
