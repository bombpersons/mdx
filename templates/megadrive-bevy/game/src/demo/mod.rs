//! The demo: replace this module with the game's features. It shows the
//! patterns worth keeping: a registered object type with tuning in an asset
//! (`player.rs`), placements and palettes from level data (`level.ron`),
//! indexed art drawn with the Mega Drive look, and rules kept apart from
//! presentation so tests can run them headless.
//!
//! The demo moves a player two pixels per tick, shows held buttons along the
//! bottom and toggles its own movement pause with Start.
use bevy::prelude::*;
use bevy::sprite::Anchor;
use engine::prelude::*;
use serde::Deserialize;
use std::sync::Arc;

pub mod player;
use player::Player;

/// `assets/demo/level.ron`.
#[derive(Deserialize)]
pub struct DemoLevel {
    pub palettes: Palettes,
    pub tileset: String,
    /// Tiles the background cycles through, as a checkerboard of 2x2-tile
    /// squares.
    pub checker: Vec<TileRef>,
    pub player_art: String,
    pub level: LevelData,
}

impl DemoLevel {
    pub fn load(root: &AssetRoot) -> Result<Self, String> {
        root.load_ron("demo/level.ron")
    }
}

#[derive(Component)]
struct Indicator(Button);

/// Rules ([`player::PlayerPlugin`]) and presentation.
pub struct DemoPlugin;

impl Plugin for DemoPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(player::PlayerPlugin)
            .add_systems(Startup, setup)
            .add_systems(Update, (dress_player, show_player, indicators, status));
    }
}

fn setup(mut commands: Commands, root: Res<AssetRoot>, screen: Res<Screen>) {
    let level = DemoLevel::load(&root).unwrap_or_else(|e| panic!("demo level: {e}"));
    let tiles = Arc::new(IndexedImage::load(&root, &level.tileset).unwrap_or_else(|e| panic!("{e}")));
    // Checkerboard of 2x2-tile squares covering the screen.
    assert!(!level.checker.is_empty(), "demo level: checker needs at least one tile");
    let mut map = TileMap::new(screen.width / 8, screen.height / 8);
    for y in 0..map.height {
        for x in 0..map.width {
            map.set(x, y, level.checker[((x / 2 + y / 2) as usize) % level.checker.len()]);
        }
    }
    let plane = TilePlane::new(tiles, map).unwrap_or_else(|e| panic!("demo background: {e}"));
    commands.spawn((Name::new("Background"), plane));
    let art = Arc::new(IndexedImage::load(&root, &level.player_art).unwrap_or_else(|e| panic!("{e}")));
    commands.insert_resource(PlayerArt(art));
    commands.insert_resource(level.palettes);
    commands.queue(SpawnLevel(level.level));
    for (index, button) in Button::ALL.into_iter().enumerate() {
        commands.spawn((
            Sprite::from_color(Color::srgb(0.3, 0.35, 0.4), Vec2::splat(10.)),
            Anchor::TOP_LEFT,
            ScreenPosition(IVec2::new(104 + index as i32 * 16, screen.height as i32 - 17)),
            Transform::from_xyz(0.0, 0.0, Depth::SPRITES_HIGH),
            Indicator(button),
        ));
    }
}

#[derive(Resource)]
struct PlayerArt(Arc<IndexedImage>);

fn dress_player(mut commands: Commands, art: Res<PlayerArt>, players: Query<(Entity, &Position), Added<Player>>) {
    for (entity, position) in &players {
        commands.entity(entity).insert((
            IndexedSprite { image: art.0.clone(), palette: 0 },
            Anchor::TOP_LEFT,
            ScreenPosition(position.0),
            Transform::from_xyz(0.0, 0.0, Depth::SPRITES_LOW),
        ));
    }
}

/// The demo has no scrolling, so level pixels are screen pixels.
fn show_player(mut players: Query<(&Position, &mut ScreenPosition), Changed<Position>>) {
    for (position, mut on_screen) in &mut players {
        on_screen.0 = position.0;
    }
}

fn indicators(pad: Res<ControllerState>, mut indicators: Query<(&Indicator, &mut Sprite)>) {
    for (indicator, mut sprite) in &mut indicators {
        sprite.color = if pad.held(indicator.0) {
            Color::srgb(1., 0.7, 0.25)
        } else {
            Color::srgb(0.3, 0.35, 0.4)
        };
    }
}

/// Makes the player's position readable through the developer MCP's
/// `game_status`.
fn status(players: Query<&Position, With<Player>>, paused: Res<player::DemoPaused>, mut status: ResMut<GameStatus>) {
    if let Ok(position) = players.single() {
        status.0.insert("player".into(), serde_json::json!([position.0.x, position.0.y]));
    }
    status.0.insert("paused".into(), paused.0.into());
}
