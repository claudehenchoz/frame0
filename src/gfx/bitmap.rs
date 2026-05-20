/// Owned BGRA (premultiplied-alpha capable) bitmap, row-major, top-down.
#[derive(Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>, // length = width * height * 4
}

impl Bitmap {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bgra: vec![0u8; (width * height * 4) as usize],
        }
    }

    pub fn row_stride(&self) -> usize {
        (self.width * 4) as usize
    }
}
