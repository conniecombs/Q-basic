pub struct SharedGraphics {
    pub active: bool,
    pub buffer: Vec<u32>,
    pub width: usize,
    pub height: usize,
    pub title: String,
    pub fg_color: u32,
    pub bg_color: u32,
    pub updated: bool,
    pub mode: i32,
}

impl SharedGraphics {
    pub fn new() -> Self {
        Self {
            active: false,
            buffer: vec![],
            width: 0,
            height: 0,
            title: String::new(),
            fg_color: 0xFFFFFF,
            bg_color: 0x000000,
            updated: false,
            mode: 0,
        }
    }
    pub fn screen(&mut self, mode: i32) {
        let (w, h) = match mode {
            1 | 7 | 13 => (320, 200),
            2 | 8 => (640, 200),
            9 => (640, 350),
            12 => (640, 480),
            _ => (640, 480),
        };
        self.mode = mode;
        self.width = w;
        self.height = h;
        self.buffer = vec![0; w * h];
        self.title = format!("QBasic Window (SCREEN {})", mode);
        self.active = true;
        self.updated = true;
    }
    pub fn cls(&mut self) {
        let bg = self.bg_color;
        for px in self.buffer.iter_mut() {
            *px = bg;
        }
        self.updated = true;
    }
    pub fn pset(&mut self, x: i32, y: i32, color: Option<u32>) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let c = color.unwrap_or(self.fg_color);
        self.buffer[y as usize * self.width + x as usize] = c;
        self.updated = true;
    }
    pub fn getpx(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.buffer[y as usize * self.width + x as usize]
    }
    pub fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Option<u32>) {
        let c = color.unwrap_or(self.fg_color);
        let (mut x0, mut y0) = (x1, y1);
        let dx = (x2 - x0).abs();
        let sx = if x0 < x2 { 1 } else { -1 };
        let dy = -(y2 - y0).abs();
        let sy = if y0 < y2 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.pset(x0, y0, Some(c));
            if x0 == x2 && y0 == y2 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
        self.updated = true;
    }
    pub fn rect(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Option<u32>, filled: bool) {
        let (xa, xb) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
        let (ya, yb) = if y1 < y2 { (y1, y2) } else { (y2, y1) };
        if filled {
            for y in ya..=yb {
                for x in xa..=xb {
                    self.pset(x, y, color);
                }
            }
        } else {
            self.line(xa, ya, xb, ya, color);
            self.line(xb, ya, xb, yb, color);
            self.line(xb, yb, xa, yb, color);
            self.line(xa, yb, xa, ya, color);
        }
        self.updated = true;
    }
    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, color: Option<u32>) {
        let c = color.unwrap_or(self.fg_color);
        let mut x = r;
        let mut y = 0;
        let mut err = 0;
        while x >= y {
            self.pset(cx + x, cy + y, Some(c));
            self.pset(cx + y, cy + x, Some(c));
            self.pset(cx - y, cy + x, Some(c));
            self.pset(cx - x, cy + y, Some(c));
            self.pset(cx - x, cy - y, Some(c));
            self.pset(cx - y, cy - x, Some(c));
            self.pset(cx + y, cy - x, Some(c));
            self.pset(cx + x, cy - y, Some(c));
            y += 1;
            err += 1 + 2 * y;
            if 2 * (err - x) + 1 > 0 {
                x -= 1;
                err += 1 - 2 * x;
            }
        }
        self.updated = true;
    }
    pub fn paint(&mut self, x: i32, y: i32, fill: u32, border: u32) {
        let target = self.getpx(x, y);
        if target == fill || target == border {
            return;
        }
        let mut stack = vec![(x, y)];
        while let Some((cx, cy)) = stack.pop() {
            if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 {
                continue;
            }
            let p = self.getpx(cx, cy);
            if p != target {
                continue;
            }
            self.pset(cx, cy, Some(fill));
            stack.push((cx + 1, cy));
            stack.push((cx - 1, cy));
            stack.push((cx, cy + 1));
            stack.push((cx, cy - 1));
        }
        self.updated = true;
    }
}

pub fn qb_color(c: u32) -> u32 {
    let idx = c & 0xFF;
    if c > 0xFF {
        return c & 0xFFFFFF;
    }
    PALETTE[idx as usize]
}

pub static PALETTE: [u32; 256] = build_palette();

const fn build_palette() -> [u32; 256] {
    let mut p = [0u32; 256];
    let ega: [u32; 16] = [
        0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555,
        0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
    ];
    let mut i = 0;
    while i < 16 {
        p[i] = ega[i];
        i += 1;
    }
    let mut k = 0;
    while k < 16 {
        let v = (k as u32 * 17) & 0xFF;
        p[16 + k] = (v << 16) | (v << 8) | v;
        k += 1;
    }
    let mut n = 0;
    while n < 224 {
        let h = (n * 360 / 224) as i32;
        let (r, g, b) = hsv_to_rgb_const(h, 100, 100);
        p[32 + n] = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        n += 1;
    }
    p
}

const fn hsv_to_rgb_const(h: i32, s: i32, v: i32) -> (i32, i32, i32) {
    let c = v * s / 100;
    let h_seg = h / 60;
    let h_rem = h - h_seg * 60;
    let x = c * (60 - abs_const(h_rem - 30) * 2) / 60;
    let (r1, g1, b1) = match h_seg {
        0 => (c, x, 0),
        1 => (x, c, 0),
        2 => (0, c, x),
        3 => (0, x, c),
        4 => (x, 0, c),
        _ => (c, 0, x),
    };
    let m = v - c;
    let r = (r1 + m) * 255 / 100;
    let g = (g1 + m) * 255 / 100;
    let b = (b1 + m) * 255 / 100;
    (clamp_const(r), clamp_const(g), clamp_const(b))
}
const fn abs_const(x: i32) -> i32 {
    if x < 0 {
        -x
    } else {
        x
    }
}
const fn clamp_const(x: i32) -> i32 {
    if x < 0 {
        0
    } else if x > 255 {
        255
    } else {
        x
    }
}
