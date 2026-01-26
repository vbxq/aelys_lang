// OpenGL-like library, uses minifb under the hood so i don't go insane

// but vbxq, why aren't you actually using opengl ?

// because this is just to showcase the FFI system
// and also because i really want to seriously work on it, i'd really like to just being able to include .h in aelys code without any problem
// the current system is okayish mais bon

// important, parts of this was written by AI, i really just to get something working
use aelys_native::*;
use minifb::{Key, Window, WindowOptions};
use std::cell::RefCell;

thread_local! {
    static WINDOW: RefCell<Option<WindowState>> = const { RefCell::new(None) };
}

struct WindowState {
    window: Window,
    buffer: Vec<u32>,
    width: usize,
    height: usize,
    clear_color: u32,
}

const DEFAULT_WIDTH: usize = 800;
const DEFAULT_HEIGHT: usize = 600;

#[aelys_module(name = "opengl", version = "0.1.0")]
mod exports {
    use super::*;

    #[aelys_export]
    pub fn init() -> i64 {
        init_with_size(DEFAULT_WIDTH as i64, DEFAULT_HEIGHT as i64)
    }

    #[aelys_export]
    pub fn init_with_size(width: i64, height: i64) -> i64 {
        let width = width.max(100) as usize;
        let height = height.max(100) as usize;

        let window = match Window::new(
            "Aelys Graphics",
            width,
            height,
            WindowOptions {
                resize: true,
                ..WindowOptions::default()
            },
        ) {
            Ok(win) => win,
            Err(_) => return 0,
        };

        let buffer = vec![0; width * height];

        let state = WindowState {
            window,
            buffer,
            width,
            height,
            clear_color: 0,
        };

        WINDOW.with(|w| {
            *w.borrow_mut() = Some(state);
        });
        1
    }

    #[aelys_export]
    pub fn clear_color(r: f64, g: f64, b: f64, _a: f64) -> i64 {
        let r_val = ((r.clamp(0.0, 1.0) * 255.0) as u32) << 16;
        let g_val = ((g.clamp(0.0, 1.0) * 255.0) as u32) << 8;
        let b_val = (b.clamp(0.0, 1.0) * 255.0) as u32;
        let color = r_val | g_val | b_val;

        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                state.clear_color = color;
                return 1;
            }
            0
        })
    }

    #[aelys_export]
    pub fn clear() -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                let color = state.clear_color;
                for pixel in &mut state.buffer {
                    *pixel = color;
                }
                return 1;
            }
            0
        })
    }

    #[aelys_export]
    pub fn draw_triangle(
        x1: i64, y1: i64,
        x2: i64, y2: i64,
        x3: i64, y3: i64,
        r: f64, g: f64, b: f64,
    ) -> i64 {
        let r_val = ((r.clamp(0.0, 1.0) * 255.0) as u32) << 16;
        let g_val = ((g.clamp(0.0, 1.0) * 255.0) as u32) << 8;
        let b_val = (b.clamp(0.0, 1.0) * 255.0) as u32;
        let color = r_val | g_val | b_val;

        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                fill_triangle(
                    &mut state.buffer,
                    state.width,
                    state.height,
                    x1 as i32, y1 as i32,
                    x2 as i32, y2 as i32,
                    x3 as i32, y3 as i32,
                    color,
                );
                return 1;
            }
            0
        })
    }

    #[aelys_export]
    pub fn draw_quad(
        x1: i64, y1: i64,
        x2: i64, y2: i64,
        x3: i64, y3: i64,
        x4: i64, y4: i64,
        r: f64, g: f64, b: f64,
    ) -> i64 {
        let r_val = ((r.clamp(0.0, 1.0) * 255.0) as u32) << 16;
        let g_val = ((g.clamp(0.0, 1.0) * 255.0) as u32) << 8;
        let b_val = (b.clamp(0.0, 1.0) * 255.0) as u32;
        let color = r_val | g_val | b_val;

        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                fill_triangle(
                    &mut state.buffer,
                    state.width,
                    state.height,
                    x1 as i32, y1 as i32,
                    x2 as i32, y2 as i32,
                    x3 as i32, y3 as i32,
                    color,
                );
                fill_triangle(
                    &mut state.buffer,
                    state.width,
                    state.height,
                    x1 as i32, y1 as i32,
                    x3 as i32, y3 as i32,
                    x4 as i32, y4 as i32,
                    color,
                );
                return 1;
            }
            0
        })
    }

    /// draw a quad with per-vertex colors (like vkcube)
    /// vertices in order: v1, v2, v3, v4 with corresponding colors c1, c2, c3, c4
    #[aelys_export]
    pub fn draw_quad_colored(
        x1: i64, y1: i64, r1: f64, g1: f64, b1: f64,
        x2: i64, y2: i64, r2: f64, g2: f64, b2: f64,
        x3: i64, y3: i64, r3: f64, g3: f64, b3: f64,
        x4: i64, y4: i64, r4: f64, g4: f64, b4: f64,
    ) -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                fill_triangle_interpolated(
                    &mut state.buffer,
                    state.width,
                    state.height,
                    x1 as i32, y1 as i32, r1 as f32, g1 as f32, b1 as f32,
                    x2 as i32, y2 as i32, r2 as f32, g2 as f32, b2 as f32,
                    x3 as i32, y3 as i32, r3 as f32, g3 as f32, b3 as f32,
                );
                fill_triangle_interpolated(
                    &mut state.buffer,
                    state.width,
                    state.height,
                    x1 as i32, y1 as i32, r1 as f32, g1 as f32, b1 as f32,
                    x3 as i32, y3 as i32, r3 as f32, g3 as f32, b3 as f32,
                    x4 as i32, y4 as i32, r4 as f32, g4 as f32, b4 as f32,
                );
                return 1;
            }
            0
        })
    }

    #[aelys_export]
    pub fn set_pixel(x: i64, y: i64, r: f64, g: f64, b: f64) -> i64 {
        let r_val = ((r.clamp(0.0, 1.0) * 255.0) as u32) << 16;
        let g_val = ((g.clamp(0.0, 1.0) * 255.0) as u32) << 8;
        let b_val = (b.clamp(0.0, 1.0) * 255.0) as u32;
        let color = r_val | g_val | b_val;

        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                let x = x as usize;
                let y = y as usize;
                if x < state.width && y < state.height {
                    state.buffer[y * state.width + x] = color;
                    return 1;
                }
            }
            0
        })
    }

    #[aelys_export]
    pub fn swap_buffers() -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow_mut().as_mut() {
                if !state.window.is_open() {
                    return 0;
                }
                if state.window.is_key_down(Key::Escape) {
                    return 0;
                }
                let _ = state.window.update_with_buffer(&state.buffer, state.width, state.height);
                return 1;
            }
            0
        })
    }

    #[aelys_export]
    pub fn is_open() -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow().as_ref() {
                if state.window.is_open() && !state.window.is_key_down(Key::Escape) {
                    return 1;
                }
            }
            0
        })
    }

    #[aelys_export]
    pub fn get_width() -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow().as_ref() {
                return state.width as i64;
            }
            0
        })
    }

    #[aelys_export]
    pub fn get_height() -> i64 {
        WINDOW.with(|w| {
            if let Some(state) = w.borrow().as_ref() {
                return state.height as i64;
            }
            0
        })
    }

    #[aelys_export]
    pub fn close() -> i64 {
        WINDOW.with(|w| {
            *w.borrow_mut() = None;
        });
        1
    }
}

fn fill_triangle(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x1: i32, y1: i32,
    x2: i32, y2: i32,
    x3: i32, y3: i32,
    color: u32,
) {
    let min_x = x1.min(x2).min(x3).max(0) as usize;
    let max_x = x1.max(x2).max(x3).min(width as i32 - 1) as usize;
    let min_y = y1.min(y2).min(y3).max(0) as usize;
    let max_y = y1.max(y2).max(y3).min(height as i32 - 1) as usize;

    fn edge(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i32 {
        (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as i32;
            let py = y as i32;

            let w0 = edge(x2, y2, x3, y3, px, py);
            let w1 = edge(x3, y3, x1, y1, px, py);
            let w2 = edge(x1, y1, x2, y2, px, py);

            if (w0 >= 0 && w1 >= 0 && w2 >= 0) || (w0 <= 0 && w1 <= 0 && w2 <= 0) {
                buffer[y * width + x] = color;
            }
        }
    }
}


fn fill_triangle_interpolated(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x1: i32, y1: i32, r1: f32, g1: f32, b1: f32,
    x2: i32, y2: i32, r2: f32, g2: f32, b2: f32,
    x3: i32, y3: i32, r3: f32, g3: f32, b3: f32,
) {
    let min_x = x1.min(x2).min(x3).max(0) as usize;
    let max_x = x1.max(x2).max(x3).min(width as i32 - 1) as usize;
    let min_y = y1.min(y2).min(y3).max(0) as usize;
    let max_y = y1.max(y2).max(y3).min(height as i32 - 1) as usize;

    let area = ((x2 - x1) * (y3 - y1) - (x3 - x1) * (y2 - y1)) as f32;
    if area.abs() < 0.001 {
        return;
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as i32;
            let py = y as i32;

            let w0 = ((x2 - px) * (y3 - py) - (x3 - px) * (y2 - py)) as f32;
            let w1 = ((x3 - px) * (y1 - py) - (x1 - px) * (y3 - py)) as f32;
            let w2 = ((x1 - px) * (y2 - py) - (x2 - px) * (y1 - py)) as f32;

            if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                let inv_area = 1.0 / area;
                let u = w0 * inv_area;
                let v = w1 * inv_area;
                let w = w2 * inv_area;
                let r = (r1 * u + r2 * v + r3 * w).clamp(0.0, 1.0);
                let g = (g1 * u + g2 * v + g3 * w).clamp(0.0, 1.0);
                let b = (b1 * u + b2 * v + b3 * w).clamp(0.0, 1.0);

                let r_val = ((r * 255.0) as u32) << 16;
                let g_val = ((g * 255.0) as u32) << 8;
                let b_val = (b * 255.0) as u32;

                buffer[y * width + x] = r_val | g_val | b_val;
            }
        }
    }
}
