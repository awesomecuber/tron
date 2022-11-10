use std::env;

use bevy::{
    prelude::*, render::camera::ScalingMode, sprite::MaterialMesh2dBundle, tasks::IoTaskPool,
};
use bevy_ggrs::*;
use ggrs::InputStatus;
use matchbox_socket::WebRtcSocket;

#[derive(Component)]
struct Player {
    handle: usize,
}

#[derive(Component, Default, Reflect, Hash)]
struct TrailSpawner {
    timer: FrameTimer,
}

#[derive(Component, Default, Reflect, Hash)]
struct Trail {
    player_handle: usize,
    death_timer: FrameTimer,
}

#[derive(Default, Reflect, Hash)]
struct FrameTimer {
    frames_left: u32,
    reset_to: u32,
}

impl FrameTimer {
    fn new(frames: u32) -> Self {
        Self {
            frames_left: frames,
            reset_to: frames,
        }
    }

    fn tick(&mut self) -> &Self {
        self.frames_left -= 1;
        if self.frames_left == 0 {
            self.frames_left = self.reset_to;
        }
        self
    }

    fn finished(&self) -> bool {
        // todo: sloppy
        self.frames_left == self.reset_to
    }
}

const PLAYER_SIZE: f32 = 0.75;
const BOARD_SIZE: f32 = 9.0;
const TRAIL_LENGTH: u32 = 80;
const TRAIL_SIZE: f32 = 0.2;
const MOVE_SPEED: f32 = 0.03;

struct GgrsConfig;

impl ggrs::Config for GgrsConfig {
    // 4-directions + fire fits easily in a single byte
    type Input = u8;
    type State = u8;
    // Matchbox' WebRtcSocket addresses are strings
    type Address = String;
}

const INPUT_LEFT: u8 = 1 << 0;
const INPUT_RIGHT: u8 = 1 << 1;
const INPUT_DASH: u8 = 1 << 2;

#[derive(Clone, Eq, PartialEq, Debug, Hash)]
enum GameState {
    Matchmaking,
    InGame,
}

fn main() {
    let mut app = App::new();

    GGRSPlugin::<GgrsConfig>::new()
        .with_input_system(input)
        .with_rollback_schedule(
            Schedule::default().with_stage(
                "ROLLBACK_STAGE",
                SystemStage::single_threaded()
                    .with_system(rotate_players)
                    .with_system(move_players_forward.after(rotate_players))
                    .with_system(spawn_trail.after(move_players_forward))
                    .with_system(kill_trail.after(spawn_trail))
                    .with_system(border_death.after(kill_trail))
                    .with_system(trail_death.after(border_death)),
            ),
        )
        .register_rollback_type::<Transform>()
        .register_rollback_type::<TrailSpawner>()
        .register_rollback_type::<Trail>()
        .build(&mut app);

    app.add_state(GameState::Matchmaking)
        .insert_resource(ClearColor(Color::rgb(0.53, 0.53, 0.53)))
        .insert_resource(WindowDescriptor {
            // fill the entire browser window
            fit_canvas_to_parent: true,
            ..default()
        })
        .add_plugins(DefaultPlugins)
        .add_system_set(
            SystemSet::on_enter(GameState::Matchmaking)
                .with_system(start_matchbox_socket)
                .with_system(setup),
        )
        .add_system_set(SystemSet::on_update(GameState::Matchmaking).with_system(wait_for_players))
        .add_system_set(SystemSet::on_enter(GameState::InGame).with_system(spawn_players))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mut camera_bundle = Camera2dBundle::default();
    camera_bundle.projection.scaling_mode = ScalingMode::FixedVertical(10.);
    commands.spawn_bundle(camera_bundle);

    commands.spawn_bundle(MaterialMesh2dBundle {
        mesh: meshes
            .add(shape::Circle::new(BOARD_SIZE / 2.).into())
            .into(),
        material: materials.add(ColorMaterial::from(Color::SEA_GREEN)),
        transform: Transform::from_translation(Vec3::new(0., 0., 0.)),
        ..default()
    });
}

fn spawn_players(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut rip: ResMut<RollbackIdProvider>,
) {
    // Player 1
    commands
        .spawn_bundle(MaterialMesh2dBundle {
            mesh: meshes
                .add(shape::Circle::new(PLAYER_SIZE / 2.).into())
                .into(),
            material: materials.add(ColorMaterial::from(Color::RED)),
            transform: Transform::from_translation(Vec3::new(-1., 0., 0.5))
                .with_rotation(Quat::from_rotation_arc_2d(Vec2::X, -Vec2::X)),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn_bundle(MaterialMesh2dBundle {
                mesh: meshes.add(shape::Circle::new(0.1).into()).into(),
                material: materials.add(ColorMaterial::from(Color::ORANGE_RED)),
                transform: Transform::from_translation(Vec3::new(PLAYER_SIZE / 2., 0., 1.)),
                ..default()
            });
        })
        .insert(Player { handle: 0 })
        .insert(TrailSpawner {
            timer: FrameTimer::new(2),
        })
        .insert(Rollback::new(rip.next_id()));

    // Player 2
    commands
        .spawn_bundle(MaterialMesh2dBundle {
            mesh: meshes
                .add(shape::Circle::new(PLAYER_SIZE / 2.).into())
                .into(),
            material: materials.add(ColorMaterial::from(Color::BLUE)),
            transform: Transform::from_translation(Vec3::new(1., 0., 0.5))
                .with_rotation(Quat::from_rotation_arc_2d(Vec2::X, Vec2::X)),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn_bundle(MaterialMesh2dBundle {
                mesh: meshes.add(shape::Circle::new(0.1).into()).into(),
                material: materials.add(ColorMaterial::from(Color::ALICE_BLUE)),
                transform: Transform::from_translation(Vec3::new(PLAYER_SIZE / 2., 0., 1.)),
                ..default()
            });
        })
        .insert(Player { handle: 1 })
        .insert(TrailSpawner {
            timer: FrameTimer::new(2),
        })
        .insert(Rollback::new(rip.next_id()));
}

fn start_matchbox_socket(mut commands: Commands) {
    let room_addr = match env::var("MATCHBOX_SERVER_ADDR") {
        Ok(val) => val,
        Err(_) => "ws://127.0.0.1:3536".into(),
    };
    let room_url = format!("{}/extreme_bevy?next=2", room_addr);
    info!("connecting to matchbox server: {:?}", room_url);
    let (socket, message_loop) = WebRtcSocket::new(room_url);

    // The message loop needs to be awaited, or nothing will happen.
    // We do this here using bevy's task system.
    IoTaskPool::get().spawn(message_loop).detach();

    commands.insert_resource(Some(socket));
}

fn wait_for_players(
    mut commands: Commands,
    mut socket: ResMut<Option<WebRtcSocket>>,
    mut state: ResMut<State<GameState>>,
) {
    let socket = socket.as_mut();

    // If there is no socket we've already started the game
    if socket.is_none() {
        return;
    }

    // Check for new connections
    socket.as_mut().unwrap().accept_new_connections();
    let players = socket.as_ref().unwrap().players();

    let num_players = 2;
    if players.len() < num_players {
        return; // wait for more players
    }

    info!("All peers have joined, going in-game");

    // create a GGRS P2P session
    let mut session_builder = ggrs::SessionBuilder::<GgrsConfig>::new()
        .with_num_players(num_players)
        .with_input_delay(2);

    for (i, player) in players.into_iter().enumerate() {
        session_builder = session_builder
            .add_player(player, i)
            .expect("failed to add player");
    }

    // move the socket out of the resource (required because GGRS takes ownership of it)
    let socket = socket.take().unwrap();

    // start the GGRS session
    let session = session_builder
        .start_p2p_session(socket)
        .expect("failed to start session");

    commands.insert_resource(session);
    commands.insert_resource(SessionType::P2PSession);

    state.set(GameState::InGame).unwrap();
}

fn input(_: In<ggrs::PlayerHandle>, keys: Res<Input<KeyCode>>) -> u8 {
    let mut input = 0u8;

    if keys.any_pressed([KeyCode::Left, KeyCode::A]) {
        input |= INPUT_LEFT
    }
    if keys.any_pressed([KeyCode::Right, KeyCode::D]) {
        input |= INPUT_RIGHT;
    }
    if keys.any_pressed([KeyCode::Space, KeyCode::Return]) {
        input |= INPUT_DASH;
    }

    input
}

fn rotate_players(
    inputs: Res<Vec<(u8, InputStatus)>>,
    mut player_query: Query<(&mut Transform, &Player)>,
) {
    for (mut transform, player) in player_query.iter_mut() {
        let (input, _) = inputs[player.handle];

        let mut angle = 0.;
        if input & INPUT_RIGHT != 0 {
            angle -= 1.;
        }
        if input & INPUT_LEFT != 0 {
            angle += 1.;
        }
        if angle == 0. {
            continue;
        }
        let move_speed = 0.13;
        transform.rotate_z(angle * move_speed)
    }
}

fn move_players_forward(
    inputs: Res<Vec<(u8, InputStatus)>>,
    mut player_query: Query<(&mut Transform, &Player)>,
) {
    for (mut transform, player) in player_query.iter_mut() {
        let (input, _) = inputs[player.handle];

        let mut speed_multiplier = 1.;
        if input & INPUT_DASH != 0 {
            speed_multiplier *= 2.;
        }

        let movement_direction = transform.rotation * Vec3::X;
        transform.translation += movement_direction * MOVE_SPEED * speed_multiplier;
    }
}

fn spawn_trail(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut trail_spawner_query: Query<(&Transform, &Player, &mut TrailSpawner)>,
) {
    for (transform, player, mut trail_spawner) in trail_spawner_query.iter_mut() {
        if trail_spawner.timer.tick().finished() {
            let color = match player.handle {
                0 => Color::ORANGE_RED,
                1 => Color::ALICE_BLUE,
                _ => panic!("invalid handle"),
            };
            commands
                .spawn_bundle(MaterialMesh2dBundle {
                    mesh: meshes
                        .add(shape::Circle::new(TRAIL_SIZE / 2.).into())
                        .into(),
                    material: materials.add(ColorMaterial::from(color)),
                    transform: Transform::from_translation(
                        transform.translation
                            - (PLAYER_SIZE + TRAIL_SIZE) / 2. * transform.local_x(),
                    ),
                    ..default()
                })
                .insert(Trail {
                    player_handle: player.handle,
                    death_timer: FrameTimer::new(TRAIL_LENGTH),
                });
        }
    }
}

fn kill_trail(mut commands: Commands, mut trail_query: Query<(Entity, &mut Trail)>) {
    for (entity, mut trail) in trail_query.iter_mut() {
        if trail.death_timer.tick().finished() {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn border_death(mut commands: Commands, player_query: Query<(Entity, &Transform), With<Player>>) {
    for (entity, transform) in player_query.iter() {
        if transform.translation.truncate().distance(Vec2::ZERO) > BOARD_SIZE / 2. {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn trail_death(
    mut commands: Commands,
    player_query: Query<(Entity, &Transform), With<Player>>,
    trail_query: Query<&Transform, With<Trail>>,
) {
    for (entity, player_transform) in player_query.iter() {
        for trail_transform in trail_query.iter() {
            let dist = player_transform
                .translation
                .truncate()
                .distance(trail_transform.translation.truncate());
            if dist < (PLAYER_SIZE + TRAIL_SIZE) / 2. {
                commands.entity(entity).despawn_recursive();
            }
        }
    }
}
