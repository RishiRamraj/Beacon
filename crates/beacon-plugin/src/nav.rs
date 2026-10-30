//! Grid pathfinding for plugins, in Rust.
//!
//! A* over a game's whole map is the expensive part of navigation: ALttP's overworld is
//! 512x512 tiles a world, and the same search in Lua took tens of milliseconds — a visible
//! hitch each time a route was planned, which in turn forced routes to be re-planned rarely,
//! so the guide led from where Link had been rather than where he was. Here it is a
//! millisecond or so, cheap enough to re-plan from the player every few frames.
//!
//! The plugin still owns the game knowledge: it decides what is walkable and fills a
//! [`Grid`] with it, and it decides what to do with the path. The host only searches, over
//! data rather than by calling back into Lua for every tile, which is what makes it fast.
//!
//! Lua: `local g = nav.grid(w, h)`, `g:fill(x, y, w, h, bytes)` (a nonzero byte is
//! walkable), `g:walkable(x, y)`, and `g:astar(sx, sy, goal, cap)` where `goal` is
//! `{ x = gx, y = gy }` or `{ rects = { { x0, y0, x1, y1 }, ... } }` (inclusive), returning
//! the tile path as `{ { x, y }, ... }` from start to goal, or nil.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};

/// A walkability grid owned by the host.
pub struct Grid {
    width: usize,
    height: usize,
    walkable: Vec<bool>,
    /// The search's working arrays, kept between searches. A fresh set is megabytes to
    /// allocate and zero for a whole-world grid — most of a search's cost — so each search
    /// instead bumps `generation`, and a cell's entries count only if stamped with it.
    scratch: RefCell<Scratch>,
}

#[derive(Default)]
struct Scratch {
    generation: u32,
    stamp: Vec<u32>,
    g: Vec<u32>,
    came: Vec<u32>,
    closed: Vec<u32>,
    open: BinaryHeap<Reverse<(usize, usize, usize)>>,
}

/// Where a search may end: one tile, or anywhere in a set of inclusive rectangles.
pub enum Goal {
    Tile(usize, usize),
    Rects(Vec<(usize, usize, usize, usize)>),
}

impl Goal {
    fn contains(&self, x: usize, y: usize) -> bool {
        match self {
            Goal::Tile(gx, gy) => x == *gx && y == *gy,
            Goal::Rects(rs) => rs
                .iter()
                .any(|&(x0, y0, x1, y1)| x >= x0 && x <= x1 && y >= y0 && y <= y1),
        }
    }

    /// Manhattan distance to the goal: admissible for four-way unit steps.
    fn heuristic(&self, x: usize, y: usize) -> usize {
        let to = |a: usize, lo: usize, hi: usize| {
            if a < lo {
                lo - a
            } else if a > hi {
                a - hi
            } else {
                0
            }
        };
        match self {
            Goal::Tile(gx, gy) => x.abs_diff(*gx) + y.abs_diff(*gy),
            Goal::Rects(rs) => rs
                .iter()
                .map(|&(x0, y0, x1, y1)| to(x, x0, x1) + to(y, y0, y1))
                .min()
                .unwrap_or(0),
        }
    }
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        Grid {
            width,
            height,
            walkable: vec![false; width * height],
            scratch: RefCell::new(Scratch::default()),
        }
    }

    pub fn walkable(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.width
            && (y as usize) < self.height
            && self.walkable[y as usize * self.width + x as usize]
    }

    /// Sets a `w` x `h` block at (`x`, `y`) from row-major bytes, nonzero meaning walkable.
    /// Cells outside the grid, and bytes past the block, are ignored.
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, bytes: &[u8]) {
        for row in 0..h {
            for col in 0..w {
                let (cx, cy) = (x + col, y + row);
                if cx < self.width && cy < self.height {
                    if let Some(&b) = bytes.get(row * w + col) {
                        self.walkable[cy * self.width + cx] = b != 0;
                    }
                }
            }
        }
    }

    /// Four-way A* from (`sx`, `sy`) to `goal`, expanding at most `cap` tiles. The path runs
    /// start to goal inclusive. `None` if the start is off the grid or no goal is reached.
    pub fn astar(&self, sx: usize, sy: usize, goal: &Goal, cap: usize) -> Option<Vec<(usize, usize)>> {
        if sx >= self.width || sy >= self.height {
            return None;
        }
        let n = self.width * self.height;
        let idx = |x: usize, y: usize| y * self.width + x;
        let mut sc = self.scratch.borrow_mut();
        let sc = &mut *sc;
        if sc.stamp.len() != n {
            sc.stamp = vec![0; n];
            sc.g = vec![0; n];
            sc.came = vec![0; n];
            sc.closed = vec![0; n];
            sc.generation = 0;
        }
        sc.generation = sc.generation.wrapping_add(1);
        if sc.generation == 0 {
            // Wrapped: every old stamp could now collide, so start the stamps over.
            sc.stamp.iter_mut().for_each(|v| *v = 0);
            sc.closed.iter_mut().for_each(|v| *v = 0);
            sc.generation = 1;
        }
        let gen = sc.generation;
        sc.open.clear();
        // Ordered by f, then by h so ties prefer the node nearer the goal (fewer expansions).
        let start = idx(sx, sy);
        sc.stamp[start] = gen;
        sc.g[start] = 0;
        sc.came[start] = u32::MAX;
        let h0 = goal.heuristic(sx, sy);
        sc.open.push(Reverse((h0, h0, start)));
        let mut expanded = 0;
        while let Some(Reverse((_, _, cur))) = sc.open.pop() {
            let (cx, cy) = (cur % self.width, cur / self.width);
            if goal.contains(cx, cy) {
                let mut path = vec![(cx, cy)];
                let mut c = cur;
                while sc.came[c] != u32::MAX {
                    c = sc.came[c] as usize;
                    path.push((c % self.width, c / self.width));
                }
                path.reverse();
                return Some(path);
            }
            if sc.closed[cur] == gen {
                continue;
            }
            sc.closed[cur] = gen;
            expanded += 1;
            if expanded > cap {
                return None;
            }
            let ng = sc.g[cur] + 1;
            let neighbours = [
                (cx as i64 + 1, cy as i64),
                (cx as i64 - 1, cy as i64),
                (cx as i64, cy as i64 + 1),
                (cx as i64, cy as i64 - 1),
            ];
            for (nx, ny) in neighbours {
                if !self.walkable(nx, ny) {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let ni = idx(nx, ny);
                if sc.stamp[ni] != gen || ng < sc.g[ni] {
                    sc.stamp[ni] = gen;
                    sc.g[ni] = ng;
                    sc.came[ni] = cur as u32;
                    let h = goal.heuristic(nx, ny);
                    sc.open.push(Reverse((ng as usize + h, h, ni)));
                }
            }
        }
        None
    }
}

impl UserData for Grid {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut(
            "fill",
            |_, this, (x, y, w, h, bytes): (usize, usize, usize, usize, mlua::String)| {
                this.fill(x, y, w, h, &bytes.as_bytes());
                Ok(())
            },
        );
        methods.add_method("walkable", |_, this, (x, y): (i64, i64)| {
            Ok(this.walkable(x, y))
        });
        methods.add_method(
            "astar",
            |lua, this, (sx, sy, goal, cap): (i64, i64, Table, Option<usize>)| {
                if sx < 0 || sy < 0 {
                    return Ok(Value::Nil);
                }
                let goal = match goal.get::<Option<Table>>("rects")? {
                    Some(rects) => {
                        let mut rs = Vec::new();
                        for r in rects.sequence_values::<Table>() {
                            let r = r?;
                            let c = |i: usize| r.get::<i64>(i).map(|v| v.max(0) as usize);
                            rs.push((c(1)?, c(2)?, c(3)?, c(4)?));
                        }
                        Goal::Rects(rs)
                    }
                    None => {
                        let gx = goal.get::<i64>("x")?;
                        let gy = goal.get::<i64>("y")?;
                        if gx < 0 || gy < 0 {
                            return Ok(Value::Nil);
                        }
                        Goal::Tile(gx as usize, gy as usize)
                    }
                };
                let cap = cap.unwrap_or(this.width * this.height);
                match this.astar(sx as usize, sy as usize, &goal, cap) {
                    None => Ok(Value::Nil),
                    Some(path) => {
                        let out = lua.create_table_with_capacity(path.len(), 0)?;
                        for (i, (x, y)) in path.into_iter().enumerate() {
                            let p = lua.create_table_with_capacity(2, 0)?;
                            p.raw_set(1, x)?;
                            p.raw_set(2, y)?;
                            out.raw_set(i + 1, p)?;
                        }
                        Ok(Value::Table(out))
                    }
                }
            },
        );
    }
}

/// Installs the `nav` table: `nav.grid(w, h)`.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let nav = lua.create_table()?;
    nav.set(
        "grid",
        lua.create_function(|_, (w, h): (usize, usize)| {
            // A cap so a typo cannot ask for gigabytes: four million cells is a 2048-square.
            if w == 0 || h == 0 || w.saturating_mul(h) > 4 << 20 {
                return Err(mlua::Error::runtime("nav.grid: size out of range"));
            }
            Ok(Grid::new(w, h))
        })?,
    )?;
    lua.globals().set("nav", nav)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(w: usize, h: usize) -> Grid {
        let mut g = Grid::new(w, h);
        g.fill(0, 0, w, h, &vec![1u8; w * h]);
        g
    }

    #[test]
    fn a_straight_path_on_open_ground_is_the_manhattan_route() {
        let g = open(10, 10);
        let p = g.astar(1, 1, &Goal::Tile(6, 1), 1000).unwrap();
        assert_eq!(p.first(), Some(&(1, 1)));
        assert_eq!(p.last(), Some(&(6, 1)));
        assert_eq!(p.len(), 6);
    }

    #[test]
    fn a_wall_is_walked_around() {
        let mut g = open(10, 10);
        // A wall down column 5, open only at the bottom row.
        g.fill(5, 0, 1, 9, &[0u8; 9]);
        let p = g.astar(2, 2, &Goal::Tile(8, 2), 1000).unwrap();
        assert!(p.iter().all(|&(x, y)| g.walkable(x as i64, y as i64)));
        assert!(p.contains(&(5, 9)), "through the only gap: {p:?}");
        // Every step is one tile.
        for w in p.windows(2) {
            assert_eq!(w[0].0.abs_diff(w[1].0) + w[0].1.abs_diff(w[1].1), 1);
        }
    }

    #[test]
    fn a_rect_goal_ends_at_its_nearest_edge() {
        let g = open(20, 20);
        let p = g.astar(2, 10, &Goal::Rects(vec![(10, 0, 19, 19)]), 10_000).unwrap();
        assert_eq!(p.last(), Some(&(10, 10)), "the first tile inside the area");
    }

    #[test]
    fn unreachable_and_over_budget_are_none() {
        let mut g = open(10, 10);
        g.fill(5, 0, 1, 10, &[0u8; 10]);
        assert!(g.astar(2, 2, &Goal::Tile(8, 2), 1000).is_none(), "walled off");
        // The kept scratch does not leak one search into the next.
        g.fill(5, 5, 1, 1, &[1]);
        assert!(g.astar(2, 2, &Goal::Tile(8, 2), 1000).is_some(), "the gap is found");
        g.fill(5, 5, 1, 1, &[0]);
        assert!(g.astar(2, 2, &Goal::Tile(8, 2), 1000).is_none(), "and gone again");
        let g = open(100, 100);
        assert!(g.astar(0, 0, &Goal::Tile(99, 99), 10).is_none(), "over budget");
    }

    #[test]
    fn the_lua_api_searches_a_filled_grid() {
        let lua = Lua::new();
        install(&lua).unwrap();
        let out: String = lua
            .load(
                r#"
                local g = nav.grid(8, 8)
                g:fill(0, 0, 8, 8, string.rep("\1", 64))
                g:fill(3, 0, 1, 7, string.rep("\0", 7))
                local p = g:astar(1, 1, { x = 6, y = 1 })
                local r = g:astar(1, 1, { rects = { { 6, 0, 7, 7 } } })
                return #p .. " " .. p[#p][1] .. "," .. p[#p][2] .. " " .. tostring(g:walkable(3, 3))
                  .. " " .. r[#r][1] .. " " .. tostring(g:astar(1, 1, { x = 3, y = 3 }))
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(out, "18 6,1 false 6 nil");
    }
}
