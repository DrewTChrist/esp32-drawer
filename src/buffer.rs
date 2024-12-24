use core::fmt::Write;

pub struct ResponseBuffer<const S: usize> {
    buf: [u8; S],
    pos: usize,
}

impl<const S: usize> ResponseBuffer<S> {
    pub fn new() -> Self {
        Self {
            buf: [0; S],
            pos: 0,
        }
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf[..self.pos]
    }

    pub fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..self.pos]
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), &str> {
        if (self.pos + bytes.len()) > self.buf.len() {
            return Err("Buffer is full");
        }
        self.buf[self.pos..(self.pos + bytes.len())].clone_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}

impl<const S: usize> Write for ResponseBuffer<S> {
    fn write_str(&mut self, in_str: &str) -> core::fmt::Result {
        let bytes = in_str.as_bytes();
        if (self.pos + bytes.len()) > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.pos..(self.pos + bytes.len())].clone_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}
