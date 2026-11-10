//! The tray icons: the app icon, and the same with a dot in the corner when a folder
//! needs attention (tray icons have no real badges). The dot is painted at runtime on
//! the decoded RGBA pixels, so only the app icon is shipped.

use tauri::image::Image;

/// The app icon (a placeholder until the real one: milestone 7c).
const ICON_PNG: &[u8] = include_bytes!("../../icons/32x32.png");
/// The dot: the UI's warning colour (`--cl-warning`), with a white ring.
const DOT: [u8; 3] = [0xc4, 0x7a, 0x14];

/// The normal icon and the one with the attention dot.
pub fn icons() -> tauri::Result<(Image<'static>, Image<'static>)> {
    let normal = Image::from_bytes(ICON_PNG)?;
    let (width, height) = (normal.width(), normal.height());
    let mut rgba = normal.rgba().to_vec();
    paint_dot(&mut rgba, width, height);
    Ok((normal, Image::new_owned(rgba, width, height)))
}

/// Paints the dot (bottom right, a third of the icon wide) over RGBA pixels.
pub fn paint_dot(rgba: &mut [u8], width: u32, height: u32) {
    let size = f64::from(width.min(height));
    let radius = size / 3.0 / 2.0 + 1.0;
    let ring = radius + size / 16.0;
    let centre = (f64::from(width) - ring, f64::from(height) - ring);
    for y in 0..height {
        for x in 0..width {
            let dx = f64::from(x) + 0.5 - centre.0;
            let dy = f64::from(y) + 0.5 - centre.1;
            let distance = (dx * dx + dy * dy).sqrt();
            let colour = if distance <= radius {
                [DOT[0], DOT[1], DOT[2], 0xff]
            } else if distance <= ring {
                [0xff, 0xff, 0xff, 0xff]
            } else {
                continue;
            };
            let Ok(index) = usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
            else {
                return;
            };
            if let Some(pixel) = rgba.get_mut(index..index + 4) {
                pixel.copy_from_slice(&colour);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_goes_in_the_bottom_right_corner() {
        let (normal, dotted) = icons().expect("icon");
        assert_eq!(
            (dotted.width(), dotted.height()),
            (normal.width(), normal.height())
        );
        let w = usize::try_from(normal.width()).expect("width");
        let h = usize::try_from(normal.height()).expect("height");
        let at = |img: &Image<'_>, x: usize, y: usize| {
            let i = (y * w + x) * 4;
            img.rgba()[i..i + 4].to_vec()
        };
        // The corner area is the dot's colour, the top left is untouched.
        let corner = at(&dotted, w - w / 6 - 1, h - h / 6 - 1);
        assert_eq!(corner, vec![DOT[0], DOT[1], DOT[2], 0xff]);
        assert_eq!(at(&dotted, 1, 1), at(&normal, 1, 1));
    }
}
