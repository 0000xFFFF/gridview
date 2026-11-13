//! Virtual masonry layout. Pure arithmetic: no widgets, no DOM. Re-laying out
//! 100k items is a couple of milliseconds, and drawing only touches the
//! handful of items that intersect the viewport (found by binary search).

use crate::scan::Item;

pub const HEADER_H: f32 = 48.0;

pub struct Group {
    pub label: String,
    pub ids: Vec<u32>,
}

#[derive(Clone, Copy)]
pub struct Placed {
    pub id: u32,
    pub y: f32,
    pub h: f32,
}

pub struct Header {
    pub y: f32,
    pub label: String,
    pub count: usize,
}

#[derive(Default)]
pub struct Layout {
    pub width: f32,
    pub columns: usize,
    pub gap: f32,
    pub col_w: f32,
    /// Per column, items in increasing y (monotonic even across groups).
    pub cols: Vec<Vec<Placed>>,
    pub headers: Vec<Header>,
    pub y_of: Vec<f32>,
    pub h_of: Vec<f32>,
    pub total_h: f32,
}

impl Layout {
    pub fn compute(
        items: &[Item],
        groups: &[Group],
        width: f32,
        columns: usize,
        gap: f32,
        show_headers: bool,
    ) -> Self {
        let columns = columns.max(1);
        let col_w = ((width - gap * (columns as f32 + 1.0)) / columns as f32).max(16.0);
        let mut cols: Vec<Vec<Placed>> = vec![Vec::new(); columns];
        let mut headers = Vec::new();
        let mut y_of = vec![0.0; items.len()];
        let mut h_of = vec![0.0; items.len()];
        let mut cursor = gap;

        for g in groups {
            if g.ids.is_empty() {
                continue;
            }
            if show_headers {
                headers.push(Header { y: cursor, label: g.label.clone(), count: g.ids.len() });
                cursor += HEADER_H;
            }
            let mut col_h = vec![cursor; columns];
            for &id in &g.ids {
                // Shortest column wins, ties go to the leftmost => the first
                // `columns` items form row 1 left-to-right, then row 2, ...
                let mut c = 0;
                for i in 1..columns {
                    if col_h[i] < col_h[c] {
                        c = i;
                    }
                }
                let h = col_w / items[id as usize].aspect();
                let y = col_h[c];
                cols[c].push(Placed { id, y, h });
                y_of[id as usize] = y;
                h_of[id as usize] = h;
                col_h[c] = y + h + gap;
            }
            cursor = col_h.iter().cloned().fold(cursor, f32::max) + gap;
        }

        Layout { width, columns, gap, col_w, cols, headers, y_of, h_of, total_h: cursor }
    }

    pub fn col_x(&self, c: usize) -> f32 {
        self.gap + c as f32 * (self.col_w + self.gap)
    }

    /// Call `f` for every item whose vertical extent intersects [y0, y1].
    pub fn visit(&self, y0: f32, y1: f32, mut f: impl FnMut(usize, &Placed)) {
        for (c, col) in self.cols.iter().enumerate() {
            let start = col.partition_point(|p| p.y + p.h < y0);
            for p in &col[start..] {
                if p.y > y1 {
                    break;
                }
                f(c, p);
            }
        }
    }

    pub fn visible_headers(&self, y0: f32, y1: f32) -> impl Iterator<Item = &Header> {
        let start = self.headers.partition_point(|h| h.y + HEADER_H < y0);
        self.headers[start..].iter().take_while(move |h| h.y <= y1)
    }

    /// The item at (or just above) scroll offset `y`, with how far into it we are.
    pub fn anchor_at(&self, y: f32) -> Option<(u32, f32)> {
        let mut best: Option<Placed> = None;
        for col in &self.cols {
            let n = col.partition_point(|p| p.y <= y);
            if n > 0 {
                let p = col[n - 1];
                if best.map_or(true, |b| p.y > b.y) {
                    best = Some(p);
                }
            }
        }
        best.map(|p| (p.id, ((y - p.y) / p.h.max(1.0)).clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Kind;
    use std::path::PathBuf;

    fn item(w: u32, h: u32) -> Item {
        Item { path: PathBuf::new(), kind: Kind::Image, modified: 0, created: 0, size: 0, w, h }
    }

    #[test]
    fn first_row_is_left_to_right() {
        let items: Vec<Item> = (0..10).map(|i| item(100 + i * 10, 100)).collect();
        let groups = vec![Group { label: "g".into(), ids: (0..10).collect() }];
        let l = Layout::compute(&items, &groups, 1000.0, 4, 4.0, false);
        // first four items land in columns 0,1,2,3 in order, all at the same y
        for c in 0..4 {
            assert_eq!(l.cols[c][0].id, c as u32);
            assert_eq!(l.cols[c][0].y, 4.0);
        }
    }

    #[test]
    fn big_layout_is_fast_and_visibility_is_small() {
        let items: Vec<Item> = (0..200_000).map(|i| item(100 + (i % 7) * 30, 100 + (i % 5) * 40)).collect();
        let groups = vec![Group { label: "g".into(), ids: (0..200_000).collect() }];
        let t = std::time::Instant::now();
        for cols in [3, 5, 8, 12] {
            let l = Layout::compute(&items, &groups, 1600.0, cols, 4.0, true);
            let mut n = 0;
            l.visit(50_000.0, 51_000.0, |_, _| n += 1);
            assert!(n > 0 && n < 200, "visible = {n}");
        }
        eprintln!("4 relayouts of 200k items: {:?}", t.elapsed());
    }
}
