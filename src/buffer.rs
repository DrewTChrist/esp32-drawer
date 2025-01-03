use core::fmt::Write;

#[derive(Clone, Copy)]
pub struct RequestBuffer<const S: usize> {
    pub buf: [u8; S],
}

impl<const S: usize> Default for RequestBuffer<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const S: usize> RequestBuffer<S> {
    pub fn new() -> Self {
        Self { buf: [0; S] }
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    pub fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

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

impl<const S: usize> Default for ResponseBuffer<S> {
    fn default() -> Self {
        Self::new()
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
