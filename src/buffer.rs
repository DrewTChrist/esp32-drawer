use core::fmt::Write;

pub struct ResponseBuffer<const S: usize> {
    pub headers: [u8; S],
    pos: usize,
}

impl<const S: usize> ResponseBuffer<S> {
    pub fn new() -> Self {
        Self {
            headers: [0; S],
            pos: 0,
        }
    }
}

impl<const S: usize> Write for ResponseBuffer<S> {
    fn write_str(&mut self, in_str: &str) -> core::fmt::Result {
        let bytes = in_str.as_bytes();
        if (self.pos + bytes.len()) > self.headers.len() {
            return Err(core::fmt::Error);
        }
        self.headers[self.pos..(self.pos + bytes.len())].clone_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}

