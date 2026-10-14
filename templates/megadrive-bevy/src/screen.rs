//! Normal Bevy game entities render on layer0 into this fixed-resolution target.
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
        Self {
            width: 320,
            height: 224,
            image: default(),
        }
    }
}

#[derive(Component)]
pub struct DisplayQuad;

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
