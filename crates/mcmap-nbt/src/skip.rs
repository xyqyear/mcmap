use crate::{
    Cursor,
    cursor::primitive_width,
    error::{Result, bad},
};
impl Cursor<'_> {
    pub fn skip(&mut self, tag: u8) -> Result<()> {
        self.skip_tree(tag, 0)
    }
    fn skip_tree(&mut self, tag: u8, depth: usize) -> Result<()> {
        if depth == 64 {
            return self.skip_iterative(tag);
        }
        match tag {
            10 => loop {
                let input = &self.bytes[self.pos..];
                let t = *input.first().ok_or_else(|| bad("truncated compound"))?;
                if t == 0 {
                    self.pos += 1;
                    return Ok(());
                }
                if t > 12 || input.len() < 3 {
                    return Err(bad("invalid compound header"));
                }
                let n = u16::from_be_bytes([input[1], input[2]]) as usize;
                let width = primitive_width(t).unwrap_or(0);
                let total = 3 + n + width;
                if total > input.len() {
                    return Err(bad("truncated named value"));
                }
                self.pos += total;
                if width == 0 {
                    self.skip_tree(t, depth + 1)?;
                }
            },
            9 => {
                let t = self.tag()?;
                let n = self.number()?.max(0) as usize;
                if n == 0 {
                    return Ok(());
                }
                if t == 0 {
                    return Err(bad("nonempty end list"));
                }
                if let Some(width) = primitive_width(t) {
                    self.take(
                        n.checked_mul(width)
                            .ok_or_else(|| bad("list length overflow"))?,
                    )?;
                } else {
                    for _ in 0..n {
                        self.skip_tree(t, depth + 1)?;
                    }
                }
                Ok(())
            }
            _ => self.leaf(tag),
        }
    }
    fn skip_iterative(&mut self, mut tag: u8) -> Result<()> {
        if tag != 9 && tag != 10 {
            return self.leaf(tag);
        }
        let mut inline = [0u64; 16];
        let mut overflow = Vec::new();
        let mut depth = 0usize;
        loop {
            let frame = match tag {
                10 => loop {
                    let input = &self.bytes[self.pos..];
                    let t = *input.first().ok_or_else(|| bad("truncated compound"))?;
                    if t == 0 {
                        self.pos += 1;
                        break None;
                    }
                    if t > 12 || input.len() < 3 {
                        return Err(bad("invalid compound header"));
                    }
                    let name = u16::from_be_bytes([input[1], input[2]]) as usize;
                    let width = primitive_width(t).unwrap_or(0);
                    let total = 3 + name + width;
                    if total > input.len() {
                        return Err(bad("truncated named value"));
                    }
                    self.pos += total;
                    if width != 0 {
                        continue;
                    }
                    if t == 9 || t == 10 {
                        tag = t;
                        break Some(u64::MAX);
                    }
                    self.leaf(t)?;
                },
                9 => {
                    tag = self.tag()?;
                    let n = self.number()?.max(0) as usize;
                    if n == 0 {
                        None
                    } else if tag == 0 {
                        return Err(bad("nonempty end list"));
                    } else if let Some(width) = primitive_width(tag) {
                        self.take(
                            n.checked_mul(width)
                                .ok_or_else(|| bad("list length overflow"))?,
                        )?;
                        None
                    } else {
                        Some((((n - 1) as u64) << 8) | tag as u64)
                    }
                }
                _ => {
                    self.leaf(tag)?;
                    None
                }
            };
            if let Some(frame) = frame {
                if depth < inline.len() {
                    inline[depth] = frame;
                } else {
                    overflow.push(frame);
                }
                depth += 1;
                continue;
            }
            loop {
                if depth == 0 {
                    return Ok(());
                }
                depth -= 1;
                let frame = if depth < inline.len() {
                    inline[depth]
                } else {
                    overflow.pop().unwrap()
                };
                if frame == u64::MAX {
                    tag = 10;
                    break;
                }
                let remaining = frame >> 8;
                if remaining != 0 {
                    tag = frame as u8;
                    let next = ((remaining - 1) << 8) | tag as u64;
                    if depth < inline.len() {
                        inline[depth] = next;
                    } else {
                        overflow.push(next);
                    }
                    depth += 1;
                    break;
                }
            }
        }
    }
}
