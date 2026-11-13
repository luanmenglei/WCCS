//! Native rendering of the xcpc-tools presentation layout. Source and asset
//! attribution are in assets/README.md. All DCs here are borrowed during paint.

use super::{Font, wide};
use std::cell::RefCell;
use windows::{
    Win32::{Foundation::*, Graphics::Gdi::*, System::SystemInformation::GetTickCount64},
    core::w,
};

pub(super) const BACKGROUND: COLORREF = COLORREF(0x050505);
pub(super) const CARD: COLORREF = COLORREF(0x080808);
pub(super) const FOREGROUND: COLORREF = COLORREF(0xf5f5f5);
pub(super) const MUTED: COLORREF = COLORREF(0xa3a3a3);
const BORDER: COLORREF = COLORREF(0x262626);
const WARNING_BITMAP: &[u8] = include_bytes!("../../assets/do-not-touch.bmp");

struct SchoolLogo {
    digest: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
thread_local! {static SCHOOL_LOGO:RefCell<Option<SchoolLogo>>=const {RefCell::new(None)};}
pub(super) fn set_logo(logo: Option<&wccs_protocol::presentation::Logo>) {
    SCHOOL_LOGO.with(|slot| {
        let mut current = slot.borrow_mut();
        if current.as_ref().map(|l| l.digest.as_str()) == logo.map(|l| l.sha256.as_str()) {
            return;
        }
        *current = logo.and_then(|logo| {
            logo.pixels().map(|(width, height, pixels)| SchoolLogo {
                digest: logo.sha256.clone(),
                width,
                height,
                pixels,
            })
        });
    });
}

pub(super) struct Layout {
    pub unit: f64,
    pub card: RECT,
    pub footer: i32,
    pub seat_right: i32,
    pub logo: RECT,
    pub labels: [(RECT, i32, i32); 12],
    pub exit: RECT,
}

pub(super) fn area(x: i32, y: i32, width: i32, height: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

impl Layout {
    pub fn new(width: i32, height: i32, _development: bool) -> Self {
        let unit = (width as f64 / 1440.)
            .min(height as f64 / 810.)
            .clamp(0.65, 2.);
        let px = |value: f64| (value * unit).round() as i32;
        let margin = px(40.);
        let header = px(112.);
        let footer = height - px(108.);
        let card_margin = if width < 900 { margin } else { px(80.) };
        let card_width = (width - 2 * card_margin).min(px(1480.));
        let card_height = px(160.);
        let card = area(
            (width - card_width) / 2,
            footer - px(24.) - card_height,
            card_width,
            card_height,
        );
        let seat_right = card.left + (card_width as f64 * 0.34).round() as i32;
        let text_left = seat_right + px(34.);
        let text_width = card.right - text_left - px(26.);
        let logo_size = px(180.).min((card.top - header - px(24.)).max(0));
        let logo = area(
            (width - logo_size) / 2,
            header + (card.top - header - logo_size) / 2,
            logo_size,
            logo_size,
        );
        let label = |x, y, width, height, size, weight| {
            (area(x, y, width, height), px(size).max(12), weight)
        };
        let labels = [
            label(
                margin,
                px(36.),
                width - 2 * margin - px(300.),
                px(50.),
                28.,
                600,
            ),
            label(text_left, card.top + px(30.), text_width, px(66.), 52., 700),
            label(
                text_left,
                card.top + px(101.),
                text_width,
                px(38.),
                27.,
                500,
            ),
            label(
                card.left + px(22.),
                card.top + px(25.),
                seat_right - card.left - px(44.),
                px(110.),
                96.,
                800,
            ),
            label(
                margin + px(110.),
                footer + px(18.),
                px(220.),
                px(30.),
                18.,
                500,
            ),
            label(
                width - margin - px(500.),
                footer + px(18.),
                px(500.),
                px(30.),
                18.,
                400,
            ),
            label(
                margin + px(240.),
                footer + px(66.),
                width - 2 * margin - px(480.),
                px(28.),
                16.,
                400,
            ),
            label(margin, footer + px(66.), px(230.), px(28.), 16., 400),
            label(
                width - margin - px(270.),
                px(40.),
                px(270.),
                px(64.),
                54.,
                400,
            ),
            label(
                width - margin - px(270.),
                px(16.),
                px(270.),
                px(22.),
                14.,
                400,
            ),
            label(
                margin,
                header,
                width - 2 * margin,
                (card.top - header).max(px(160.)),
                14.,
                400,
            ),
            label(margin, footer + px(18.), px(90.), px(30.), 18., 700),
        ];
        Self {
            unit,
            card,
            footer,
            seat_right,
            logo,
            labels,
            exit: area(
                width - margin - px(210.),
                footer + px(60.),
                px(210.),
                px(36.),
            ),
        }
    }
}

fn fill(dc: HDC, rect: &RECT, color: COLORREF) {
    // SAFETY: dc is borrowed from the active paint; stock objects are not owned.
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, rect, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
}

fn line(dc: HDC, points: &[POINT]) {
    // SAFETY: point slice outlives the synchronous draw and dc is live.
    unsafe {
        let _ = Polyline(dc, points);
    }
}

fn school(dc: HDC, rect: RECT) {
    let size = rect.right - rect.left;
    let point = |x: i32, y: i32| POINT {
        x: rect.left + x * size / 24,
        y: rect.top + y * size / 24,
    };
    // SAFETY: owned pen is restored out of the borrowed DC before deletion.
    unsafe {
        let pen = CreatePen(PS_SOLID, (size / 120).max(1), COLORREF(0x414141));
        if pen.is_invalid() {
            return;
        }
        let old = SelectObject(dc, pen.into());
        line(
            dc,
            &[
                point(2, 10),
                point(12, 4),
                point(22, 10),
                point(12, 16),
                point(2, 10),
            ],
        );
        line(
            dc,
            &[
                point(6, 13),
                point(6, 18),
                point(12, 21),
                point(18, 18),
                point(18, 13),
            ],
        );
        line(dc, &[point(22, 10), point(22, 17)]);
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
    }
}

fn draw_scene(dc: HDC, rect: RECT, development: bool, animate: bool) {
    fill(dc, &rect, BACKGROUND);
    let layout = Layout::new(rect.right, rect.bottom, development);
    // SAFETY: GetTickCount64 is monotonic and used only for decorative motion.
    let elapsed = if animate {
        (unsafe { GetTickCount64() }) as f64 / 1000.
    } else {
        0.
    };
    // Deterministic particles keep resizing stable. Their opacity and very slow
    // drift follow the reference's restrained white particle field.
    // SAFETY: dc is live for this paint. Stock pens/brushes are borrowed and every
    // selection is restored before returning. Geometry is bounded by the client.
    unsafe {
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        for index in 0..300u32 {
            let seed = index.wrapping_mul(747796405).wrapping_add(2891336453);
            let x = (seed & 65535) as f64 / 65535.;
            let y = (seed.rotate_left(13) & 65535) as f64 / 65535.;
            let phase = seed as f64 / u32::MAX as f64 * std::f64::consts::TAU;
            let x = (x * rect.right as f64 + (elapsed * 0.035 + phase).sin() * 18. * layout.unit)
                as i32;
            let y = (y * rect.bottom as f64 + (elapsed * 0.025 + phase).cos() * 14. * layout.unit)
                as i32;
            // Header and footer are quiet opaque surfaces for native accessible text.
            if y < (112. * layout.unit) as i32 || y > layout.footer {
                continue;
            }
            let radius = ((1. + (seed % 3) as f64 * 0.45) * layout.unit)
                .round()
                .max(1.) as i32;
            let gray = 55 + seed % 100;
            SetDCBrushColor(dc, COLORREF(gray * 0x010101));
            let _ = Ellipse(dc, x - radius, y - radius, x + radius, y + radius);
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        let old_pen = SelectObject(dc, GetStockObject(DC_PEN));
        let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCPenColor(dc, BORDER);
        SetDCBrushColor(dc, CARD);
        let radius = (12. * layout.unit) as i32;
        let _ = RoundRect(
            dc,
            layout.card.left,
            layout.card.top,
            layout.card.right,
            layout.card.bottom,
            radius,
            radius,
        );
        line(
            dc,
            &[
                POINT {
                    x: layout.seat_right,
                    y: layout.card.top,
                },
                POINT {
                    x: layout.seat_right,
                    y: layout.card.bottom,
                },
            ],
        );
        let margin = (40. * layout.unit) as i32;
        line(
            dc,
            &[
                POINT {
                    x: margin,
                    y: layout.footer,
                },
                POINT {
                    x: rect.right - margin,
                    y: layout.footer,
                },
            ],
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
    }
    SCHOOL_LOGO.with(|slot| {
        let logo = slot.borrow();
        if let Some(logo) = logo.as_ref() {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: logo.width as i32,
                    biHeight: logo.height as i32,
                    biPlanes: 1,
                    biBitCount: 24,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: protocol validation proves stride, dimensions and buffer length.
            // This synchronous GDI call borrows pixels and retains no pointer or handle.
            unsafe {
                StretchDIBits(
                    dc,
                    layout.logo.left,
                    layout.logo.top,
                    layout.logo.right - layout.logo.left,
                    layout.logo.bottom - layout.logo.top,
                    0,
                    0,
                    logo.width as i32,
                    logo.height as i32,
                    Some(logo.pixels.as_ptr().cast()),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
        } else {
            school(dc, layout.logo);
        }
    });
}

pub(super) fn paint(dc: HDC, rect: RECT, development: bool, animate: bool) {
    // SAFETY: temporary compatible DC/bitmap are owned here. Restore the bitmap
    // before deleting it, then release the memory DC. Parent clips child controls.
    unsafe {
        let buffer = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, rect.right.max(1), rect.bottom.max(1));
        if buffer.is_invalid() || bitmap.is_invalid() {
            if !bitmap.is_invalid() {
                let _ = DeleteObject(bitmap.into());
            }
            if !buffer.is_invalid() {
                let _ = DeleteDC(buffer);
            }
            draw_scene(dc, rect, development, animate);
            return;
        }
        let previous = SelectObject(buffer, bitmap.into());
        draw_scene(buffer, rect, development, animate);
        let _ = BitBlt(
            dc,
            0,
            0,
            rect.right,
            rect.bottom,
            Some(buffer),
            0,
            0,
            SRCCOPY,
        );
        SelectObject(buffer, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(buffer);
    }
}

fn warning_text(dc: HDC, value: &str, mut rect: RECT, size: i32, weight: i32, color: COLORREF) {
    // SAFETY: temporary font is deselected before Font drops. Text buffer and rect
    // outlive DrawTextW. No borrowed object is retained by Windows.
    unsafe {
        let font = Font(CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        ));
        if font.0.is_invalid() {
            return;
        }
        let previous = SelectObject(dc, font.0.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, color);
        let mut text = wide(value.as_ref());
        text.pop();
        DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, previous);
    }
}

pub(super) fn paint_warning(dc: HDC, rect: RECT) {
    fill(dc, &rect, COLORREF(0x020202));
    let layout = Layout::new(rect.right, rect.bottom, false);
    let px = |n: f64| (n * layout.unit).round() as i32;
    let panel_width = (rect.right - px(80.)).min(px(1280.));
    let panel_height = (rect.bottom - px(100.)).min(px(620.));
    let panel = area(
        (rect.right - panel_width) / 2,
        (rect.bottom - panel_height) / 2,
        panel_width,
        panel_height,
    );
    // SAFETY: stock brush/pen are borrowed, restored before returning. Bitmap bytes
    // are a compile-time 420×420, 24-bit bottom-up BMP with a 54-byte header.
    unsafe {
        let pen = SelectObject(dc, GetStockObject(DC_PEN));
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCPenColor(dc, BORDER);
        SetDCBrushColor(dc, BACKGROUND);
        let _ = RoundRect(
            dc,
            panel.left,
            panel.top,
            panel.right,
            panel.bottom,
            px(12.),
            px(12.),
        );
        SelectObject(dc, brush);
        SelectObject(dc, pen);
        let side = px(340.).min(panel_height - px(160.)).max(1);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 420,
                biHeight: 420,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let previous_mode = SetStretchBltMode(dc, HALFTONE);
        let mut previous_origin = POINT::default();
        let _ = SetBrushOrgEx(dc, 0, 0, Some(&mut previous_origin));
        StretchDIBits(
            dc,
            (rect.right - side) / 2,
            panel.top + px(28.),
            side,
            side,
            0,
            0,
            420,
            420,
            Some(WARNING_BITMAP[54..].as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        SetStretchBltMode(dc, STRETCH_BLT_MODE(previous_mode));
        let _ = SetBrushOrgEx(dc, previous_origin.x, previous_origin.y, None);
        let text_top = panel.top + px(28.) + side + px(10.);
        let english_size = px(34.)
            .min(((panel_width - px(40.)) as f64 / 23.).round() as i32)
            .max(12);
        warning_text(
            dc,
            "DON'T TOUCH MOUSE AND KEYBOARD",
            area(
                panel.left + px(20.),
                text_top,
                panel_width - px(40.),
                px(58.),
            ),
            english_size,
            700,
            FOREGROUND,
        );
        warning_text(
            dc,
            "请勿触碰鼠标和键盘",
            area(
                panel.left + px(20.),
                text_top + px(64.),
                panel_width - px(40.),
                px(54.),
            ),
            px(34.),
            400,
            MUTED,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_identity_stays_inside_landscape_portrait_and_small_windows() {
        for (width, height) in [
            (640, 480),
            (800, 600),
            (1080, 1920),
            (1920, 1080),
            (3840, 2160),
        ] {
            for development in [false, true] {
                let layout = Layout::new(width, height, development);
                assert!(layout.logo.bottom < layout.card.top);
                assert!(layout.card.bottom < layout.footer);
                for (index, (rect, size, _)) in layout.labels.iter().enumerate() {
                    if !development && [6, 7].contains(&index) {
                        continue;
                    }
                    assert!(rect.left >= 0 && rect.top >= 0);
                    assert!(
                        rect.right <= width && rect.bottom <= height,
                        "{width}×{height} label {index}"
                    );
                    assert!(rect.right > rect.left && rect.bottom > rect.top);
                    assert!(*size >= 12);
                }
                assert!(layout.labels[3].0.right < layout.labels[1].0.left);
                assert!(layout.labels[0].0.right < layout.labels[8].0.left);
            }
        }
    }

    #[test]
    fn embedded_warning_bitmap_matches_the_native_dib_contract() {
        assert_eq!(&WARNING_BITMAP[..2], b"BM");
        let number =
            |offset| u32::from_le_bytes(WARNING_BITMAP[offset..offset + 4].try_into().unwrap());
        assert_eq!(number(10), 54);
        assert_eq!(number(18), 420);
        assert_eq!(number(22), 420);
        assert_eq!(&WARNING_BITMAP[26..30], &[1, 0, 24, 0]);
        assert_eq!(number(30), 0);
        assert_eq!(WARNING_BITMAP.len(), 54 + 420 * 420 * 3);
    }
}
