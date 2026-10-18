//! Normal Bevy game entities render on layer 0 into this fixed-resolution
//! target, which a second camera scales into the window.
//!
//! The target's world coordinates are Bevy's (origin at the centre, +y up).
//! Mega Drive code thinks in screen pixels from the top-left, +y down:
//! [`ScreenPosition`] places an entity that way, and [`Depth`] orders
//! planes and sprites the way the original's priorities do.
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::render_resource::TextureFormat,
};

#[derive(Resource)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub image: Handle<Image>,
}
impl Default for Screen {
    fn default() -> Self {
        Self::new(320, 224)
    }
}
impl Screen {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, image: default() }
    }
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32)
    }
    /// The world position of screen pixel `pixel` (top-left origin, +y down).
    pub fn to_world(&self, pixel: Vec2) -> Vec2 {
        Vec2::new(pixel.x - self.width as f32 / 2.0, self.height as f32 / 2.0 - pixel.y)
    }
}

/// Where an entity's top-left corner sits on screen, in whole pixels from
/// the screen's top-left, +y down. Use it with `Anchor::TOP_LEFT` sprites;
/// [`place_on_screen`] writes the `Transform`'s x and y and keeps its z.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenPosition(pub IVec2);

/// Draw order (`Transform` z) matching the original's priorities: the
/// backdrop, then each layer's low-priority tiles, then high-priority ones.
/// Within a band, larger z draws in front, so sprites can add a small
/// fraction for their order among themselves.
pub struct Depth;
impl Depth {
    pub const BACKDROP: f32 = 0.0;
    pub const PLANE_B_LOW: f32 = 1.0;
    pub const PLANE_A_LOW: f32 = 2.0;
    pub const SPRITES_LOW: f32 = 3.0;
    pub const PLANE_B_HIGH: f32 = 4.0;
    pub const PLANE_A_HIGH: f32 = 5.0;
    pub const SPRITES_HIGH: f32 = 6.0;
    /// Menus and debug overlays drawn over the game.
    pub const OVERLAY: f32 = 50.0;
}

pub fn place_on_screen(
    screen: Res<Screen>,
    mut placed: Query<(&ScreenPosition, &mut Transform), Changed<ScreenPosition>>,
) {
    for (position, mut transform) in &mut placed {
        let world = screen.to_world(position.0.as_vec2());
        transform.translation.x = world.x;
        transform.translation.y = world.y;
    }
}

#[derive(Component)]
pub struct DisplayQuad;

/// The camera drawing layer 0 into the screen texture. Its clear colour is
/// the backdrop ([`crate::look::Palettes::backdrop`] sets it).
#[derive(Component)]
pub struct ScreenCamera;

/// No window: only the game texture is rendered (`--headless`).
#[derive(Resource, Clone, Copy)]
pub struct Headless(pub bool);

pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut screen: ResMut<Screen>,
    headless: Res<Headless>,
) {
    let mut image = Image::new_target_texture(
        screen.width,
        screen.height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
    screen.image = images.add(image);
    commands.spawn((
        ScreenCamera,
        Camera2d,
        bevy::render::view::Msaa::Off,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.06, 0.08, 0.12)),
            ..default()
        },
        RenderTarget::Image(screen.image.clone().into()),
        RenderLayers::layer(0),
    ));
    if headless.0 {
        return;
    }
    commands.spawn((
        Camera2d,
        bevy::render::view::Msaa::Off,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderLayers::layer(1),
    ));
    commands.spawn((
        Sprite::from_image(screen.image.clone()),
        Transform::default(),
        DisplayQuad,
        RenderLayers::layer(1),
    ));
}

pub fn fit_scale(window: Vec2, screen: Vec2) -> f32 {
    (window.x / screen.x).min(window.y / screen.y).max(0.0)
}

pub fn fit_window(
    windows: Query<&Window>,
    screen: Res<Screen>,
    mut display: Query<&mut Transform, With<DisplayQuad>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = fit_scale(
        Vec2::new(window.width(), window.height()),
        Vec2::new(screen.width as f32, screen.height as f32),
    );
    for mut transform in &mut display {
        transform.scale = Vec3::splat(scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_pixels_map_to_centred_world_coordinates() {
        let screen = Screen::new(320, 224);
        assert_eq!(screen.to_world(Vec2::ZERO), Vec2::new(-160.0, 112.0));
        assert_eq!(screen.to_world(Vec2::new(160.0, 112.0)), Vec2::ZERO);
    }
    #[test]
    fn wide_tall_and_small_windows_fit_without_stretching() {
        let screen = Vec2::new(320., 224.);
        for window in [
            Vec2::new(1600., 900.),
            Vec2::new(300., 900.),
            Vec2::new(160., 112.),
        ] {
            let fitted = screen * fit_scale(window, screen);
            assert!(fitted.x <= window.x + 0.001 && fitted.y <= window.y + 0.001);
            assert!((fitted.x / fitted.y - 320. / 224.).abs() < 0.0001);
        }
    }
}
