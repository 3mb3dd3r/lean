//! The launcher's look: accent palettes, fonts, egui visuals, and the custom painting the
//! window uses (ambient gradients, glows, icons, the LEAN wordmark, switches).
//!
//! Everything here is drawn by egui itself. Nothing animates while the window is idle, so it
//! costs nothing between frames; the only continuous redraws are short hover animations.

use eframe::egui::{
    self, epaint, pos2, vec2, Color32, FontData, FontDefinitions, FontFamily, FontId, Mesh, Painter, Pos2, Rect,
    Response, Rounding, Sense, Shape, Stroke, TextStyle, TextureHandle, Ui, Vec2,
};

// ---------------------------------------------------------------- palette

pub const BG: Color32 = Color32::from_rgb(8, 8, 10);
pub const TEXT: Color32 = Color32::from_rgb(244, 244, 246);
pub const TEXT_2: Color32 = Color32::from_rgb(161, 161, 170);
pub const TEXT_3: Color32 = Color32::from_rgb(141, 141, 151);
pub const GREEN: Color32 = Color32::from_rgb(61, 220, 132);
pub const AMBER: Color32 = Color32::from_rgb(245, 165, 36);
pub const ERROR: Color32 = Color32::from_rgb(255, 128, 140);

/// White at `a` (0 to 255) alpha.
pub const fn white(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    Red,
    Blue,
    Purple,
    Mono,
}

impl Accent {
    pub const ALL: [Accent; 4] = [Accent::Red, Accent::Blue, Accent::Purple, Accent::Mono];

    pub fn from_name(s: Option<&str>) -> Self {
        match s {
            Some("blue") => Accent::Blue,
            Some("purple") => Accent::Purple,
            Some("mono") => Accent::Mono,
            _ => Accent::Red,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Accent::Red => "red",
            Accent::Blue => "blue",
            Accent::Purple => "purple",
            Accent::Mono => "mono",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Accent::Red => "Red",
            Accent::Blue => "Blue",
            Accent::Purple => "Purple",
            Accent::Mono => "Mono",
        }
    }
    pub fn palette(self) -> Palette {
        let rgba = Color32::from_rgba_unmultiplied;
        match self {
            Accent::Red => Palette {
                c: Color32::from_rgb(255, 61, 82),
                dark: Color32::from_rgb(122, 12, 28),
                deep: rgba(170, 16, 40, 140),
                btn_end: Color32::from_rgb(240, 38, 63),
                soft: rgba(255, 61, 82, 36),
                soft_glow: rgba(255, 61, 82, 46),
                glow: rgba(255, 61, 82, 115),
                icon: Color32::from_rgb(255, 107, 123),
                ink: Color32::WHITE,
            },
            Accent::Blue => Palette {
                c: Color32::from_rgb(77, 141, 255),
                dark: Color32::from_rgb(19, 48, 110),
                deep: rgba(28, 70, 190, 140),
                btn_end: Color32::from_rgb(47, 111, 232),
                soft: rgba(77, 141, 255, 36),
                soft_glow: rgba(77, 141, 255, 46),
                glow: rgba(77, 141, 255, 115),
                icon: Color32::from_rgb(126, 171, 255),
                ink: Color32::WHITE,
            },
            Accent::Purple => Palette {
                c: Color32::from_rgb(168, 107, 255),
                dark: Color32::from_rgb(62, 26, 122),
                deep: rgba(100, 40, 190, 140),
                btn_end: Color32::from_rgb(139, 79, 240),
                soft: rgba(168, 107, 255, 36),
                soft_glow: rgba(168, 107, 255, 46),
                glow: rgba(168, 107, 255, 115),
                icon: Color32::from_rgb(196, 155, 255),
                ink: Color32::WHITE,
            },
            Accent::Mono => Palette {
                c: Color32::from_rgb(230, 230, 234),
                dark: Color32::from_rgb(58, 58, 66),
                deep: rgba(110, 110, 120, 90),
                btn_end: Color32::from_rgb(92, 92, 102),
                soft: rgba(255, 255, 255, 20),
                soft_glow: rgba(255, 255, 255, 20),
                glow: rgba(255, 255, 255, 56),
                icon: Color32::from_rgb(230, 230, 234),
                ink: Color32::from_rgb(11, 11, 13),
            },
        }
    }
}

/// One accent's colours. Alpha colours are given unmultiplied above and stored premultiplied.
#[derive(Clone, Copy)]
pub struct Palette {
    pub c: Color32,
    pub dark: Color32,
    pub deep: Color32,
    pub btn_end: Color32,
    pub soft: Color32,
    pub soft_glow: Color32,
    pub glow: Color32,
    pub icon: Color32,
    /// Text on a solid accent fill.
    pub ink: Color32,
}

// ---------------------------------------------------------------- fonts and visuals

pub const LIGHT: &str = "light";
pub const SEMIBOLD: &str = "semibold";

pub fn light(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(LIGHT.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Segoe UI from the Windows fonts folder (every Windows install has it; it is not bundled).
/// Falls back to egui's own font where a file is missing.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                defs.font_data.insert(key.to_string(), FontData::from_owned(bytes));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let light = load("segoe-light", "segoeuisl.ttf") || load("segoe-light", "segoeuil.ttf");
    let semi = load("segoe-semibold", "seguisb.ttf");
    let base = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: Option<&str>| -> Vec<String> {
        let mut v: Vec<String> = first.map(|f| vec![f.to_string()]).unwrap_or_default();
        v.extend(base.iter().cloned());
        v
    };
    if regular {
        defs.families.insert(FontFamily::Proportional, with(Some("segoe")));
    }
    let regular_or_none = regular.then_some("segoe");
    defs.families.insert(FontFamily::Name(LIGHT.into()), with(if light { Some("segoe-light") } else { regular_or_none }));
    defs.families.insert(FontFamily::Name(SEMIBOLD.into()), with(if semi { Some("segoe-semibold") } else { regular_or_none }));
    ctx.set_fonts(defs);
}

pub fn apply_style(ctx: &egui::Context, accent: Accent) {
    let p = accent.palette();
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Small, regular(12.0)),
        (TextStyle::Body, regular(14.0)),
        (TextStyle::Button, regular(14.0)),
        (TextStyle::Heading, semibold(26.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = vec2(10.0, 10.0);
    style.spacing.button_padding = vec2(14.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.slider_width = 260.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = None;
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = Color32::from_rgb(18, 18, 22);
    v.window_stroke = Stroke::new(1.0_f32, white(26));
    v.window_rounding = Rounding::same(16.0);
    v.menu_rounding = Rounding::same(12.0);
    v.window_shadow = epaint::Shadow { offset: vec2(0.0, 18.0), blur: 50.0, spread: 0.0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = epaint::Shadow { offset: vec2(0.0, 10.0), blur: 30.0, spread: 0.0, color: Color32::from_black_alpha(140) };
    v.extreme_bg_color = Color32::from_black_alpha(90);
    v.faint_bg_color = white(6);
    v.code_bg_color = white(8);
    v.hyperlink_color = p.icon;
    v.selection.bg_fill = mul_alpha(p.c, 0.55);
    v.selection.stroke = Stroke::new(1.0_f32, p.c);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    let w = &mut v.widgets;
    let r = Rounding::same(10.0);
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, white(18));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(214, 214, 220));
    w.noninteractive.rounding = r;
    w.inactive.bg_fill = white(14);
    w.inactive.weak_bg_fill = white(10);
    w.inactive.bg_stroke = Stroke::new(1.0_f32, white(30));
    w.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(228, 228, 232));
    w.inactive.rounding = r;
    w.hovered.bg_fill = white(22);
    w.hovered.weak_bg_fill = white(18);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, white(60));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = white(30);
    w.active.weak_bg_fill = white(26);
    w.active.bg_stroke = Stroke::new(1.0_f32, p.c);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.rounding = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    ctx.set_style(style);
}

pub fn mul_alpha(c: Color32, f: f32) -> Color32 {
    c.gamma_multiply(f)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}


// ---------------------------------------------------------------- gradients and glow

/// A soft elliptical glow: `color` at the centre fading to nothing at `radius`.
pub fn radial(painter: &Painter, center: Pos2, radius: Vec2, color: Color32) {
    const SEG: usize = 48;
    const RINGS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.62), (0.7, 0.22), (1.0, 0.0)];
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for &(r, k) in &RINGS[1..] {
        for i in 0..SEG {
            let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(center + vec2(a.cos() * radius.x * r, a.sin() * radius.y * r), mul_alpha(color, k));
        }
    }
    for i in 0..SEG as u32 {
        let j = (i + 1) % SEG as u32;
        mesh.add_triangle(0, 1 + i, 1 + j);
        for ring in 0..2u32 {
            let a = 1 + ring * SEG as u32;
            let b = a + SEG as u32;
            mesh.add_triangle(a + i, a + j, b + i);
            mesh.add_triangle(a + j, b + j, b + i);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Fills a rounded rect with a colour that varies left to right (`f(t)`, t from 0 to 1).
/// Built from thin vertical strips, so any curve works, not only linear blends.
pub fn h_gradient(painter: &Painter, rect: Rect, radius: f32, f: impl Fn(f32) -> Color32) {
    let r = radius.min(rect.height() / 2.0).min(rect.width() / 2.0);
    let mut xs: Vec<f32> = Vec::new();
    for i in 0..=8 {
        xs.push(rect.left() + r * i as f32 / 8.0);
    }
    let mid_steps = ((rect.width() - 2.0 * r) / 10.0).ceil().max(1.0) as usize;
    for i in 1..mid_steps {
        xs.push(rect.left() + r + (rect.width() - 2.0 * r) * i as f32 / mid_steps as f32);
    }
    for i in 0..=8 {
        xs.push(rect.right() - r + r * i as f32 / 8.0);
    }
    let inset = |x: f32| -> f32 {
        let dx = if x < rect.left() + r {
            rect.left() + r - x
        } else if x > rect.right() - r {
            x - (rect.right() - r)
        } else {
            0.0
        };
        r - (r * r - dx.min(r) * dx.min(r)).max(0.0).sqrt()
    };
    let mut mesh = Mesh::default();
    for &x in &xs {
        let c = f((x - rect.left()) / rect.width());
        let h = inset(x);
        mesh.colored_vertex(pos2(x, rect.top() + h), c);
        mesh.colored_vertex(pos2(x, rect.bottom() - h), c);
    }
    for i in 0..(xs.len() as u32 - 1) {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// A soft accent-coloured glow around `rect`.
pub fn glow(painter: &Painter, rect: Rect, radius: f32, color: Color32, blur: f32) {
    let shadow = epaint::Shadow { offset: Vec2::ZERO, blur, spread: 0.0, color };
    painter.add(shadow.as_shape(rect, radius));
}

/// The window's ambient background: near black with accent light from two corners.
pub fn background(painter: &Painter, screen: Rect, p: &Palette) {
    painter.rect_filled(screen, 0.0, BG);
    radial(painter, screen.left_top() + vec2(-40.0, -60.0), vec2(700.0, 520.0), mul_alpha(p.deep, 0.42));
    radial(painter, screen.right_bottom() + vec2(60.0, 80.0), vec2(600.0, 440.0), mul_alpha(p.deep, 0.36));
    radial(painter, pos2(screen.right() - 160.0, screen.top() + 30.0), vec2(420.0, 300.0), white(6));
    // Two faint arcs, like light catching the edge of a lens.
    let arc = |c: Pos2, r: f32, from: f32, to: f32, color: Color32| {
        let pts: Vec<Pos2> = (0..=64)
            .map(|i| {
                let a = from + (to - from) * i as f32 / 64.0;
                c + vec2(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(Shape::line(pts, Stroke::new(1.0_f32, color)));
    };
    let pi = std::f32::consts::PI;
    arc(pos2(screen.right() + 420.0, screen.top() + 120.0), 700.0, 0.62 * pi, 1.12 * pi, white(12));
    arc(pos2(screen.left() - 300.0, screen.bottom() + 420.0), 620.0, 1.55 * pi, 1.95 * pi, mul_alpha(p.c, 0.12));
}

/// A glass card: faint white fill, thin white outline.
pub fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(white(8))
        .stroke(Stroke::new(1.0_f32, white(22)))
        .rounding(16.0)
        .inner_margin(egui::Margin::same(18.0))
}

/// Small letter-spaced section label ("ACCOUNT").
pub fn section_label(ui: &mut Ui, text: &str) {
    let spaced: String = text.chars().flat_map(|c| [c, '\u{200A}']).collect();
    ui.label(egui::RichText::new(spaced).font(regular(11.5)).color(TEXT_3));
}

// ---------------------------------------------------------------- the Play button

pub enum PlayLabel<'a> {
    Play,
    Text(&'a str),
}

/// The big Play button: a flat accent fill, with a brighter spot that follows the cursor on hover.
pub fn play_button(ui: &mut Ui, p: &Palette, label: PlayLabel, height: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let ctx = ui.ctx();
    let id = resp.id;
    let hover = ctx.animate_bool_with_time(id.with("hover"), resp.hovered(), 0.18);
    let press = ctx.animate_bool_with_time(id.with("press"), resp.is_pointer_button_down_on(), 0.08);
    // Where the light sits: under the cursor while hovered, back at the right end otherwise.
    let target = match resp.hover_pos() {
        Some(pos) => ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
        None => 0.9,
    };
    let focus = ctx.animate_value_with_time(id.with("focus"), target, 0.22);
    let radius = height / 2.0 - 6.0;
    let painter = ui.painter();

    glow(painter, rect.shrink(6.0), radius, mul_alpha(p.glow, 0.15 + 0.3 * hover), 26.0 + 14.0 * hover);
    // Solid base first, so the glow underneath doesn't show through.
    painter.rect_filled(rect, radius, Color32::from_rgb(16, 16, 19));
    // At rest one flat accent color; hovered, a soft brighter spot follows the cursor.
    let base = lerp_color(p.dark, p.btn_end, 0.72);
    h_gradient(painter, rect, radius, |t| {
        let d = (t - focus) / 0.3;
        let c = lerp_color(base, p.c, 0.85 * hover * (-d * d).exp());
        lerp_color(c, Color32::BLACK, 0.12 * press)
    });
    if hover > 0.0 {
        // A faint highlight right under the cursor, clipped to the button's shape.
        let x = rect.left() + focus * rect.width();
        let y = resp.hover_pos().map(|p| p.y).unwrap_or(rect.center().y).clamp(rect.top(), rect.bottom());
        radial_in_rounded(painter, pos2(x, y), vec2(height * 1.6, height * 1.1), mul_alpha(white(30), hover), rect, radius);
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, white(28 + (22.0 * hover) as u8)));

    let text_color = Color32::WHITE;
    match label {
        PlayLabel::Play => {
            let font = semibold(24.0);
            let galley = painter.layout_no_wrap("Play".into(), font, text_color);
            let icon = 20.0;
            let gap = 14.0;
            let total = icon + gap + galley.size().x;
            let left = rect.center().x - total / 2.0;
            let icon_rect = Rect::from_center_size(pos2(left + icon / 2.0, rect.center().y), Vec2::splat(icon));
            icons::paint(painter, icons::Icon::Play, icon_rect, text_color);
            painter.galley(pos2(left + icon + gap, rect.center().y - galley.size().y / 2.0), galley, text_color);
        }
        PlayLabel::Text(t) => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, t, semibold(19.0), text_color);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Like [`radial`], with its outer ring pulled inside a rounded rect so it never spills past it.
fn radial_in_rounded(painter: &Painter, center: Pos2, radius: Vec2, color: Color32, rect: Rect, r: f32) {
    let inner = rect.shrink(r);
    let clamp = |p: Pos2| -> Pos2 {
        let q = pos2(p.x.clamp(inner.left(), inner.right()), p.y.clamp(inner.top(), inner.bottom()));
        let d = p - q;
        let len = d.length();
        if len > r {
            q + d / len * r
        } else {
            p
        }
    };
    const SEG: usize = 40;
    const RINGS: [(f32, f32); 3] = [(0.4, 0.5), (0.75, 0.16), (1.0, 0.0)];
    let mut mesh = Mesh::default();
    mesh.colored_vertex(clamp(center), color);
    for &(rr, k) in &RINGS {
        for i in 0..SEG {
            let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(clamp(center + vec2(a.cos() * radius.x * rr, a.sin() * radius.y * rr)), mul_alpha(color, k));
        }
    }
    for i in 0..SEG as u32 {
        let j = (i + 1) % SEG as u32;
        mesh.add_triangle(0, 1 + i, 1 + j);
        for ring in 0..2u32 {
            let a = 1 + ring * SEG as u32;
            let b = a + SEG as u32;
            mesh.add_triangle(a + i, a + j, b + i);
            mesh.add_triangle(a + j, b + j, b + i);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The launch progress, drawn in the Play button's place.
pub fn progress_bar(ui: &mut Ui, p: &Palette, frac: f32, status: &str, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    let radius = height / 2.0 - 6.0;
    glow(painter, rect.shrink(4.0), radius, p.soft_glow, 40.0);
    painter.rect_filled(rect, radius, Color32::from_rgb(16, 16, 19));
    painter.rect_filled(rect, radius, white(9));
    painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, white(34)));
    let inner = rect.shrink2(vec2(28.0, 0.0));
    let text_y = rect.center().y - 9.0;
    painter.text(pos2(inner.left(), text_y), egui::Align2::LEFT_CENTER, status, regular(15.0), TEXT);
    painter.text(pos2(inner.right(), text_y), egui::Align2::RIGHT_CENTER, format!("{:.0}%", frac * 100.0), regular(14.0), TEXT_2);
    let bar = Rect::from_min_size(pos2(inner.left(), rect.center().y + 10.0), vec2(inner.width(), 4.0));
    painter.rect_filled(bar, 2.0, white(20));
    let fill = Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), bar.height()));
    if fill.width() > 1.0 {
        glow(painter, fill, 2.0, p.glow, 12.0);
        h_gradient(painter, fill, 2.0, |t| lerp_color(p.dark, p.c, t));
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

// ---------------------------------------------------------------- switch

/// An on/off switch. Returns the response; `on` flips when it is clicked.
pub fn switch(ui: &mut Ui, on: &mut bool, p: &Palette) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    paint_switch(ui, rect, *on, resp.hovered(), resp.id, p);
    resp
}

/// Draws a switch in `rect` (36 x 20) without handling input: for tiles that are clickable as a whole.
pub fn paint_switch(ui: &Ui, rect: Rect, on: bool, hovered: bool, id: egui::Id, p: &Palette) {
    let t = ui.ctx().animate_bool_with_time(id.with("switch"), on, 0.14);
    let painter = ui.painter();
    let r = rect.height() / 2.0;
    if t > 0.0 {
        glow(painter, rect, r, mul_alpha(p.glow, 0.5 * t), 14.0);
    }
    painter.rect_filled(rect, r, lerp_color(white(if hovered { 46 } else { 36 }), p.c, t));
    let knob_x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    // White knob, except on the light mono track, where it is dark.
    let knob = if t > 0.5 { p.ink } else { Color32::WHITE };
    painter.circle_filled(pos2(knob_x, rect.center().y), r - 2.5, knob);
}

// ---------------------------------------------------------------- logo and wordmark

/// The logo as two textures: its own colours (for red) and a white copy tinted for other accents.
pub struct Logo {
    pub original: TextureHandle,
    pub white: TextureHandle,
}

pub const LOGO_PNG: &[u8] = include_bytes!("../assets/lean-logo.png");

pub fn decode_logo() -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(LOGO_PNG));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

pub fn window_icon() -> Option<egui::IconData> {
    let (w, h, rgba) = decode_logo()?;
    Some(egui::IconData { rgba, width: w as u32, height: h as u32 })
}

impl Logo {
    pub fn load(ctx: &egui::Context) -> Option<Self> {
        let (w, h, rgba) = decode_logo()?;
        let original = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        let whitened: Vec<u8> = rgba
            .chunks(4)
            .flat_map(|c| {
                let v = c[0].max(c[1]).max(c[2]);
                // Lift the darker faces a little so the shape still reads once tinted.
                let v = (v as f32).powf(0.85) * 255f32.powf(0.15);
                let v = v.min(255.0) as u8;
                [v, v, v, c[3]]
            })
            .collect();
        let white = egui::ColorImage::from_rgba_unmultiplied([w, h], &whitened);
        Some(Self {
            original: ctx.load_texture("logo", original, egui::TextureOptions::LINEAR),
            white: ctx.load_texture("logo-white", white, egui::TextureOptions::LINEAR),
        })
    }

    pub fn paint(&self, ui: &Ui, rect: Rect, accent: Accent) {
        let (tex, tint) = match accent {
            Accent::Red => (&self.original, Color32::WHITE),
            other => (&self.white, other.palette().c),
        };
        egui::Image::new((tex.id(), rect.size())).tint(tint).paint_at(ui, rect);
    }
}

/// Minecraft-style 5 x 7 glyphs for the wordmark.
const GLYPHS: [(char, [&str; 7]); 4] = [
    ('L', ["#....", "#....", "#....", "#....", "#....", "#....", "#####"]),
    ('E', ["#####", "#....", "#....", "####.", "#....", "#....", "#####"]),
    ('A', [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('N', ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"]),
];

/// "LEAN" in blocky Minecraft-like letters, `px` points per pixel, centred on `center`.
pub fn wordmark(painter: &Painter, center: Pos2, px: f32, p: &Palette) {
    let text = "LEAN";
    let gap = 3.0; // pixels between letters
    let width = text.len() as f32 * 5.0 + (text.len() as f32 - 1.0) * gap;
    let origin = center - vec2(width * px / 2.0, 3.5 * px);
    // Pixel shadow, like Minecraft's text shadow, in the accent's dark shade.
    for pass in 0..2 {
        let (offset, color) = if pass == 0 { (vec2(px * 0.6, px * 0.6), mul_alpha(p.dark, 0.9)) } else { (Vec2::ZERO, Color32::WHITE) };
        for (i, ch) in text.chars().enumerate() {
            let Some((_, rows)) = GLYPHS.iter().find(|(c, _)| *c == ch) else { continue };
            let lx = origin.x + i as f32 * (5.0 + gap) * px;
            for (y, row) in rows.iter().enumerate() {
                for (x, b) in row.bytes().enumerate() {
                    if b != b'#' {
                        continue;
                    }
                    let c = if pass == 0 {
                        color
                    } else {
                        // Top rows pure white, easing to a soft grey at the bottom.
                        lerp_color(Color32::WHITE, Color32::from_rgb(196, 196, 204), y as f32 / 6.0)
                    };
                    let min = pos2(lx + x as f32 * px, origin.y + y as f32 * px) + offset;
                    // A hair of overlap avoids seams between neighbouring pixels.
                    painter.rect_filled(Rect::from_min_size(min, Vec2::splat(px + 0.3)), 0.0, c);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- icons

pub mod icons {
    use super::*;

    #[derive(Clone, Copy)]
    pub enum Icon {
        Home,
        Profiles,
        Settings,
        User,
        Cube,
        Play,
        Chevron,
        Minimize,
        Maximize,
        Restore,
        Close,
        Bolt,
        Mouse,
        Compass,
        Flask,
        Pin,
        Chip,
        Spark,
        Cup,
        Power,
        Plus,
        File,
    }

    /// Draws `icon` (designed on a 24 x 24 grid, 1.6 stroke) into `rect`.
    pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
        let s = rect.width().min(rect.height()) / 24.0;
        let o = rect.center() - vec2(12.0 * s, 12.0 * s);
        let p = |x: f32, y: f32| o + vec2(x * s, y * s);
        let stroke = Stroke::new(1.6 * s.max(0.75), color);
        let line = |pts: &[(f32, f32)]| {
            painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
        };
        let closed = |pts: &[(f32, f32)]| {
            painter.add(Shape::closed_line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
        };
        let circle = |x: f32, y: f32, r: f32| {
            painter.circle_stroke(p(x, y), r * s, stroke);
        };
        let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = (from + (to - from) * i as f32 / 24.0).to_radians();
                    p(cx + a.cos() * r, cy + a.sin() * r)
                })
                .collect();
            painter.add(Shape::line(pts, stroke));
        };
        match icon {
            Icon::Home => {
                line(&[(3.0, 10.5), (12.0, 3.0), (21.0, 10.5)]);
                line(&[(5.0, 9.5), (5.0, 21.0), (19.0, 21.0), (19.0, 9.5)]);
                line(&[(10.0, 21.0), (10.0, 15.0), (14.0, 15.0), (14.0, 21.0)]);
            }
            Icon::Profiles => {
                closed(&[(12.0, 3.0), (21.0, 8.0), (12.0, 13.0), (3.0, 8.0)]);
                line(&[(3.0, 13.0), (12.0, 18.0), (21.0, 13.0)]);
            }
            Icon::Settings => {
                line(&[(4.0, 6.0), (14.0, 6.0)]);
                line(&[(18.0, 6.0), (20.0, 6.0)]);
                line(&[(4.0, 12.0), (8.0, 12.0)]);
                line(&[(12.0, 12.0), (20.0, 12.0)]);
                line(&[(4.0, 18.0), (16.0, 18.0)]);
                circle(16.0, 6.0, 2.0);
                circle(10.0, 12.0, 2.0);
                circle(18.0, 18.0, 2.0);
            }
            Icon::User => {
                circle(12.0, 8.0, 4.0);
                line(&[(4.0, 21.0), (5.5, 18.0), (8.0, 16.0), (12.0, 15.0), (16.0, 16.0), (18.5, 18.0), (20.0, 21.0)]);
            }
            Icon::Cube => {
                closed(&[(12.0, 2.5), (20.5, 7.25), (20.5, 16.75), (12.0, 21.5), (3.5, 16.75), (3.5, 7.25)]);
                line(&[(3.5, 7.25), (12.0, 12.0), (20.5, 7.25)]);
                line(&[(12.0, 12.0), (12.0, 21.5)]);
            }
            Icon::Play => {
                painter.add(Shape::convex_polygon(vec![p(6.5, 4.0), p(20.5, 12.0), p(6.5, 20.0)], color, Stroke::NONE));
            }
            Icon::Chevron => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
            Icon::Minimize => line(&[(5.0, 12.0), (19.0, 12.0)]),
            Icon::Maximize => closed(&[(6.0, 6.0), (18.0, 6.0), (18.0, 18.0), (6.0, 18.0)]),
            Icon::Restore => {
                closed(&[(5.0, 9.0), (15.0, 9.0), (15.0, 19.0), (5.0, 19.0)]);
                line(&[(9.0, 9.0), (9.0, 5.0), (19.0, 5.0), (19.0, 15.0), (15.0, 15.0)]);
            }
            Icon::Close => {
                line(&[(6.0, 6.0), (18.0, 18.0)]);
                line(&[(18.0, 6.0), (6.0, 18.0)]);
            }
            Icon::Bolt => closed(&[(13.0, 2.0), (4.0, 14.0), (11.0, 14.0), (10.0, 22.0), (19.0, 10.0), (12.0, 10.0)]),
            Icon::Mouse => {
                painter.rect_stroke(Rect::from_min_max(p(6.0, 3.0), p(18.0, 21.0)), 6.0 * s, stroke);
                line(&[(12.0, 3.0), (12.0, 9.0)]);
            }
            Icon::Compass => {
                circle(12.0, 12.0, 9.0);
                closed(&[(15.5, 8.5), (13.5, 13.5), (8.5, 15.5), (10.5, 10.5)]);
            }
            Icon::Flask => {
                line(&[(9.0, 3.0), (15.0, 3.0)]);
                line(&[
                    (10.0, 3.0),
                    (10.0, 8.5),
                    (5.5, 17.0),
                    (5.8, 19.6),
                    (7.7, 20.7),
                    (16.3, 20.7),
                    (18.2, 19.6),
                    (18.5, 17.0),
                    (14.0, 8.5),
                    (14.0, 3.0),
                ]);
                line(&[(7.3, 14.0), (16.7, 14.0)]);
            }
            Icon::Pin => {
                closed(&[
                    (12.0, 21.0),
                    (8.0, 16.5),
                    (5.6, 12.6),
                    (5.0, 9.5),
                    (5.9, 6.0),
                    (8.4, 3.5),
                    (12.0, 2.5),
                    (15.6, 3.5),
                    (18.1, 6.0),
                    (19.0, 9.5),
                    (18.4, 12.6),
                    (16.0, 16.5),
                ]);
                circle(12.0, 9.5, 2.5);
            }
            Icon::Chip => {
                painter.rect_stroke(Rect::from_min_max(p(6.0, 6.0), p(18.0, 18.0)), 2.0 * s, stroke);
                for v in [9.0, 15.0] {
                    line(&[(v, 2.0), (v, 6.0)]);
                    line(&[(v, 18.0), (v, 22.0)]);
                    line(&[(2.0, v), (6.0, v)]);
                    line(&[(18.0, v), (22.0, v)]);
                }
            }
            Icon::Spark => {
                line(&[(12.0, 3.0), (12.0, 7.0)]);
                line(&[(12.0, 17.0), (12.0, 21.0)]);
                line(&[(3.0, 12.0), (7.0, 12.0)]);
                line(&[(17.0, 12.0), (21.0, 12.0)]);
                line(&[(5.6, 5.6), (8.4, 8.4)]);
                line(&[(15.6, 15.6), (18.4, 18.4)]);
                line(&[(18.4, 5.6), (15.6, 8.4)]);
                line(&[(8.4, 15.6), (5.6, 18.4)]);
            }
            Icon::Cup => {
                closed(&[(5.0, 9.0), (16.0, 9.0), (16.0, 14.0), (14.5, 17.5), (11.0, 19.0), (10.0, 19.0), (6.5, 17.5), (5.0, 14.0)]);
                line(&[(16.0, 11.0), (17.8, 11.0), (19.5, 12.4), (19.5, 14.6), (17.8, 16.0), (16.0, 16.0)]);
                line(&[(8.5, 3.0), (9.5, 4.5), (8.5, 6.0)]);
                line(&[(12.5, 3.0), (13.5, 4.5), (12.5, 6.0)]);
            }
            Icon::Power => {
                arc(12.0, 13.0, 8.0, -50.0, 230.0);
                line(&[(12.0, 3.0), (12.0, 12.0)]);
            }
            Icon::Plus => {
                line(&[(12.0, 5.0), (12.0, 19.0)]);
                line(&[(5.0, 12.0), (19.0, 12.0)]);
            }
            Icon::File => {
                closed(&[(14.0, 3.0), (7.0, 3.0), (5.0, 5.0), (5.0, 19.0), (7.0, 21.0), (17.0, 21.0), (19.0, 19.0), (19.0, 8.0)]);
                line(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]);
            }
        }
    }
}
