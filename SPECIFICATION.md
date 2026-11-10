# Open Cycling World

**Cross-Platform Indoor Cycling Simulation**

Project Description and Technical Architecture

Version 1.0 – November 2026

---

## 1. Product Vision

Open Cycling World is a cross-platform indoor cycling simulation for Linux, macOS and Windows.

The application connects real bicycle hardware with an interactive virtual 3D world. Users can ride alone or together with other participants on real geographic routes, train, take part in races, and individually customize their bicycles and their virtual characters.

The geographic foundation is OpenStreetMap data. From this data, locally usable 3D roads, landscapes and routes are generated.

Functionally, the simulation is modeled on Zwift and MyWhoosh, but it shall have an independent, modular, Rust-based architecture.

### 1.1 Main Goals

- Native desktop application for Linux, macOS and Windows.
- Complete implementation in Rust, wherever technically reasonable.
- 3D rendering via Bevy and wgpu.
- Support for Bluetooth smart trainers and bicycle sensors.
- Realistic bicycle and route physics.
- Real-time multiplayer with many participants riding simultaneously.
- Integrated voice communication.
- Bicycle and character editor.
- Generation of geographic 3D worlds from OpenStreetMap.
- Local rendering and local caching of route data.
- Offline-capable single-player rides on downloaded routes.
- Modular extensibility for future features.

---

## 2. Feature Scope

### 2.1 User Management and Start Screen

The application has a central dashboard.

Features:

- User account and profile.
- Rider statistics.
- Training history.
- Personal bests.
- Available routes.
- Current multiplayer sessions.
- Friends and groups.
- Scheduled events.
- Device management.
- Personal settings.

The application shall also provide a local training mode without requiring sign-in.

### 2.2 Virtual Cycling World

The 3D world is based on real geographic data.

Examples:

- City streets.
- Country roads.
- Forest trails.
- Cycle paths.
- Mountain routes.
- Alpine passes.
- Coastal roads.
- Custom-defined circuits.

The virtual world is reconstructed from geographic data.

The presentation includes:

- 3D terrain.
- Road surfaces.
- Road markings.
- Curves and gradients.
- Forests and vegetation.
- Buildings and bridges.
- Rivers and lakes.
- Lighting and shadows.
- Weather and times of day.

The graphical presentation does not need to match reality exactly. However, route layout and elevation profiles shall match the geographic data as closely as possible.

### 2.3 Real-Time Multiplayer

Multiplayer is a core part of the application and is taken into account in the architecture from the start.

Features:

- Shared rides.
- Group rides.
- Races.
- Time trials.
- Training groups.
- Private sessions.
- Public events.
- Friends lists.
- Invitations.
- Live leaderboards.
- Real-time positions of other riders.
- Power and speed comparisons.

Each participant is represented by their own avatar.

The position, speed, heading and animation of other participants are continuously synchronized.

#### Multiplayer Requirements

- Server-side authority over race state.
- Client-side prediction for the local rider.
- Interpolation of other participants.
- Latency compensation.
- Reconnection after network interruptions.
- Protection against manipulated power data.
- Separation of training data and visual presentation.

The simulation must not depend on the client's frame rate.

### 2.4 Voice Communication

The application includes integrated real-time voice communication.

Features:

- Group voice channels.
- Private voice rooms.
- Voice communication during shared rides.
- Push-to-talk.
- Permanently enableable (open) microphone.
- Microphone and speaker selection.
- Muting of individual participants.
- Global mute.
- Volume control.
- Automatic echo cancellation.
- Noise suppression.
- Connection status.

Additionally, spatial audio shall be possible.

For example, voices of riders who are virtually behind the local rider may sound quieter than voices of immediately adjacent riders.

Voice communication is technically separated from the actual game state.

### 2.5 Bicycle Editor

Each user can configure their virtual bicycle.

Features:

- Selection of bicycle type:
  - Road bike.
  - Gravel bike.
  - Mountain bike.
  - Time trial bike.
- Selection of frame and color.
- Wheels.
- Tires.
- Handlebars.
- Saddle.
- Attachments / accessories.
- Custom paint schemes.
- Component unlocking.
- 3D preview.

The bicycle configuration consists of reusable components.

Each component has its own properties, such as:

- Weight.
- Aerodynamics.
- Rolling resistance.
- Geometry.
- Visual appearance.

Visual properties must be separated from physical properties.

Not every cosmetic part may automatically produce a performance advantage.

### 2.6 Character Editor

The user can individually design their rider.

Features:

- Body model.
- Gender or body representation.
- Skin color.
- Facial features.
- Hairstyles.
- Clothing.
- Shoes.
- Helmet.
- Glasses.
- Gloves.
- Jerseys.
- Shorts / pants.
- Animations.

Characters use a shared skeleton and animation system.

Animation is driven by cadence, speed and riding state.

Planned animations:

- Normal pedaling.
- Sprint.
- Climbing.
- Out-of-the-saddle (standing) pedaling.
- Coasting.
- Cornering.
- Finish-line arrival.
- Victory animation.

Bicycle and character must be correctly attached to each other in the 3D presentation.

---

## 3. Overall Technical Stack

### 3.1 Programming Language and Engine

- **Primary programming language:** Rust
- **3D engine:** Bevy
- **Graphics backend:** wgpu

Bevy handles:

- 3D scenes.
- Rendering.
- Cameras.
- Lighting.
- Shadows.
- Materials.
- Animations.
- Entity Component System.
- Asset management.
- Game states.
- User interface.

wgpu provides the cross-platform GPU abstraction.

Graphics backends:

- Vulkan.
- Metal.
- Direct3D 12.

### 3.2 User Interface

Recommended technology: **Bevy UI**.

The user interface shall be connected as directly as possible to the engine state.

For more complex development and diagnostic interfaces, **egui** may be used additionally.

The interface is divided into modules:

- Dashboard.
- Training view.
- Multiplayer lobby.
- Device management.
- Bicycle editor.
- Character editor.
- Settings.
- Route selection.

### 3.3 Audio

Two separate audio systems:

**Game audio:**

- Background music.
- Ambient sounds.
- Bicycle sounds.
- Wind.
- Tires.
- Freewheel.
- Gear shifting.

**Voice communication:**

- WebRTC.
- Opus audio codec.
- LiveKit SFU.
- Microphone and speaker selection.
- Echo cancellation.
- Noise suppression.

Voice communication is provided via a separate service.

### 3.4 Hardware Communication

Bluetooth Low Energy:

- Fitness Machine Service (FTMS).
- Cycling Power Service.
- Cycling Speed and Cadence.
- Heart Rate Service.

Recommended Rust library: **btleplug**.

ANT+ is added via a separate hardware backend.

The device abstraction must not have a direct dependency on the 3D engine.

---

## 4. OpenStreetMap and Local 3D World

### 4.1 Data Source

OpenStreetMap is the primary source of geographic information.

Data used:

- Roads.
- Cycle paths.
- Paths / tracks.
- Road categories.
- Intersections.
- Bridges.
- Tunnels.
- Buildings.
- Water bodies.
- Forests.
- Land use.
- Settlement areas.
- Place information.

Data format: **OSM PBF**.

Data sources for regional extracts:

- Geofabrik.
- Custom OSM data extracts.
- Optionally, other OSM data providers.

### 4.2 Local Data Processing

The application shall not use pre-rendered map images as the basis of the 3D world.

Instead, a custom data pipeline is developed.

Process:

1. Select region.
2. Download OSM PBF data.
3. Extract relevant geographic objects.
4. Transform coordinates.
5. Reconstruct road courses.
6. Determine or estimate road widths.
7. Generate road surfaces.
8. Generate terrain.
9. Place buildings and vegetation.
10. Generate meshes and textures.
11. Store data locally.
12. Load world into Bevy.

### 4.3 Terrain and Elevation Data

OpenStreetMap does not contain precise elevation information with full coverage.

Therefore, an additional digital elevation model (DEM) source is required.

Possible sources:

- Copernicus DEM.
- SRTM.
- Regional open elevation models.

The following are generated from the elevation data:

- Elevation profiles.
- Terrain meshes.
- Climbs.
- Descents.
- Mountain passes.
- Valleys.
- Elevation differences.

Bridges and tunnels must be handled separately, since their height must not simply equal the terrain height.

### 4.4 Road Reconstruction

Each road is converted into a 3D road segment.

Properties:

- Geographic centerline.
- Road width.
- Surface material.
- Gradient.
- Curve radius.
- Road marking.
- Connectivity information.
- Road category.

The road surface is generated from the line geometries.

The road connections simultaneously form the basis for route guidance and navigation.

### 4.5 Local Rendering

The application uses its own Bevy rendering system.

Geometry is divided into spatial tiles.

Recommended structure:

- Region.
- Area.
- World tile.
- Road section.
- Terrain mesh.
- Building group.
- Vegetation group.

Only the required areas are loaded into memory.

While riding, neighboring tiles are loaded in the background and areas no longer needed are unloaded.

Goals:

- No continuous downloads during a ride.
- Offline reuse of downloaded regions.
- Bounded memory consumption.
- GPU-accelerated rendering.
- LOD for distant objects.
- Frustum culling.
- Instancing for vegetation and recurring objects.

### 4.6 Data Packages

Regions are stored as self-contained data packages.

Example:

```
worlds/europe/germany/berlin/
```

Contents:

- Geographic source data.
- Terrain data.
- Road meshes.
- Building meshes.
- Vegetation.
- Materials.
- Metadata.
- Version information.

Packages shall be signed, versioned and updatable.

---

## 5. Bicycle Physics and Training Simulation

Bicycle physics is a standalone Rust module.

**Inputs:**

- Power in watts.
- Cadence.
- Rider weight.
- Bicycle weight.
- Gradient.
- Speed.
- Wind.
- Rolling resistance.
- Air resistance.

**Outputs:**

- Speed.
- Acceleration.
- Position.
- Energy expenditure.
- Resistance.
- Riding state.

The fundamental resistive forces:

- Gravitational force.
- Rolling resistance.
- Aerodynamic drag.
- Inertial (acceleration) resistance.

The simulation uses a fixed time step.

Trainer control translates the virtual road gradient into a suitable resistance value for compatible smart trainers.

Training data is stored independently of rendering.

---

## 6. Real-Time Multiplayer Architecture

### 6.1 Client-Server Model

The application uses a server-authoritative model.

The client transmits:

- Power.
- Cadence.
- Device information.
- Inputs.
- Riding state.

The server computes or validates:

- Rider state.
- Position.
- Speed.
- Race state.
- Rankings.
- Group membership.
- Events.

The server then distributes updated states to the participants.

### 6.2 Network Protocol

Recommended technologies:

- Tokio as the asynchronous runtime.
- Quinn for QUIC-based communication.
- Protobuf or MessagePack for structured messages.
- Server-side state management.

Unreliable datagrams may be used for position transmission, while login, session management and important events are transmitted reliably.

### 6.3 Synchronization

The simulation must account for the following aspects:

- Differing frame rates.
- Differing network latencies.
- Packet loss.
- Connection drops.
- Reconnection.
- Server-side timestamps.
- Interpolation of other riders.
- Client-side prediction.

The multiplayer logic is developed independently of the graphics engine.

---

## 7. Voice Chat

Recommended infrastructure: **LiveKit**.

LiveKit provides a WebRTC SFU service.

Features:

- Voice rooms.
- Participant management.
- Audio transmission.
- Dynamic subscription to audio tracks.
- Push-to-talk.
- Group communication.

Voice communication is **not** transmitted over the QUIC game-state connection.

Voice chat and game server are separate services with separate scaling mechanisms.

Spatial volume and distance effects are implemented on the client side.

---

## 8. Bicycle and Character Editor

### 8.1 Asset Pipeline

Recommended 3D format: **glTF 2.0**.

Models are created, for example, with Blender.

Import process:

1. Create model.
2. Define materials.
3. Create skeleton and animations.
4. Export glTF.
5. Validate assets.
6. Import into Bevy.
7. Register components.

### 8.2 Bicycle Components

Bicycles consist of modular parts.

Example:

- Frame.
- Fork.
- Handlebars.
- Wheels.
- Tires.
- Saddle.
- Pedals.
- Drivetrain.
- Accessories.

Each component has a unique ID and a configuration.

### 8.3 Characters

Characters use a shared rig.

Body, clothing and accessories are loaded modularly.

Animations are driven by the riding state.

---

## 9. Backend and Infrastructure

The desktop application is the actual simulation client.

Server services handle only tasks that must be centrally managed or synchronized between participants.

Recommended backend stack:

| Area | Technology |
|---|---|
| Language | Rust |
| Async runtime | Tokio |
| HTTP API | Axum |
| Real-time networking | Quinn / QUIC |
| Data format | Protobuf |
| Database | PostgreSQL |
| Cache | Redis (optional) |
| Voice | LiveKit |
| Authentication | OIDC |
| Telemetry | OpenTelemetry |
| Containers | Docker |
| Deployment | Linux servers |

Backend modules:

- Authentication Service.
- User Service.
- Profile Service.
- Device Service.
- Multiplayer Session Service.
- Race Management.
- Friends and Groups.
- World Metadata Service.
- Asset Distribution.
- Training History.
- Voice Room Management.

For the first version, a modular backend monolith is recommended. Splitting into many microservices only makes sense once there is an actual scaling need.

---

## 10. Cargo Workspace and Repository Structure

Recommended structure:

```
open-cycling-world/
│
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
│
├── apps/
│   ├── desktop/
│   └── world-builder/
│
├── crates/
│   ├── core/
│   ├── simulation/
│   ├── cycling-physics/
│   ├── cycling-protocol/
│   ├── device-manager/
│   ├── bluetooth-ftms/
│   ├── ant-plus/
│   ├── world/
│   ├── osm-import/
│   ├── terrain/
│   ├── road-generator/
│   ├── renderer/
│   ├── avatar/
│   ├── bicycle/
│   ├── character-editor/
│   ├── bicycle-editor/
│   ├── multiplayer/
│   ├── voice/
│   ├── training/
│   ├── telemetry/
│   └── platform/
│
├── services/
│   ├── api/
│   ├── game-server/
│   ├── account-service/
│   └── world-distribution/
│
├── assets/
│   ├── bicycles/
│   ├── characters/
│   ├── animations/
│   ├── materials/
│   ├── sounds/
│   └── shaders/
│
├── worlds/
│
├── tools/
│   ├── asset-validator/
│   ├── osm-converter/
│   └── performance-test/
│
├── infrastructure/
│   ├── docker/
│   └── deployment/
│
└── .github/
    └── workflows/
```

---

## 11. Cross-Platform Build System

Target platforms:

- Linux x86_64.
- Linux ARM64 (optional).
- macOS Apple Silicon.
- macOS Intel (optional).
- Windows x86_64.

Each platform gets a native CI runner.

### Build Process

1. Install Rust toolchain.
2. Resolve dependencies.
3. Format code.
4. Run Clippy.
5. Run unit tests.
6. Run integration tests.
7. Create release build.
8. Package assets.
9. Create installers.
10. Publish artifacts.

### Packaging

**Linux:**

- AppImage.
- Flatpak.
- Optionally native packages.

**macOS:**

- App bundle.
- Code signing.
- Apple notarization.
- DMG.

**Windows:**

- EXE/MSI.
- Code signing.
- Installer.

Build and release pipelines must be automated for all three operating systems.

---

## 12. Security and Privacy

Particularly important for multiplayer and voice communication:

- Encrypted network connections.
- Authentication.
- Server-side authorization.
- Protection against manipulated training data.
- Protection against replay attacks.
- Rate limiting.
- Secure token management.
- Microphone permissions.
- Visible microphone activity.
- Muting.
- Privacy-friendly telemetry.

Personal data, voice data and training data must be handled separately.

---

## 13. Licensing and Open-Source Strategy

Preferred licenses:

- MIT.
- Apache 2.0.

OpenStreetMap data is licensed under the **ODbL**.

This results in specific requirements for distributing databases and derived databases.

The app must visibly credit OpenStreetMap as a source.

Geographic data and rendering software must be considered separately from a licensing perspective.

Use of the public OSM tile servers must not be confused with the free use of the raw data.

For production, therefore, dedicated regional data packages or a suitable self-hosted data server are planned.

Before commercial release, a review of data and asset licenses is required.

---

## 14. Development Phases

### Phase 1 – Technical Foundation

- Cargo workspace.
- Bevy application.
- Linux/macOS/Windows builds.
- Rendering.
- Camera.
- Simple test route.

**Result:** A launchable cross-platform application.

### Phase 2 – Bicycle and Sensors

- Bluetooth scanning.
- FTMS.
- Power measurement.
- Cadence.
- Heart rate.
- Bicycle physics.
- Virtual resistance.

**Result:** Ride with a real smart trainer.

### Phase 3 – OpenStreetMap World

- PBF import.
- Road extraction.
- Elevation model.
- Terrain generation.
- Road meshes.
- Local caching.
- Regional data packages.

**Result:** Ride a geographically realistic route locally in 3D.

### Phase 4 – Avatar and Bicycle Editor

- Bicycle models.
- Character models.
- Components.
- Animations.
- Configuration and persistence.

**Result:** Individually customizable riders.

### Phase 5 – Multiplayer

- Accounts.
- Sessions.
- Real-time synchronization.
- Other riders.
- Group rides.
- Races.
- Leaderboards.

**Result:** Shared virtual rides.

### Phase 6 – Voice Chat

- LiveKit.
- WebRTC.
- Microphone management.
- Push-to-talk.
- Voice rooms.
- Spatial audio.

**Result:** Shared rides with voice communication.

### Phase 7 – Production Readiness

- Performance optimization.
- Large regions.
- Weather and times of day.
- Training history.
- Events.
- Error monitoring.
- Installers and updates.
- Security audits.

**Result:** A release-ready indoor cycling platform.

---

## 15. Quality Goals

The application shall pursue the following technical goals:

- 60 FPS target on suitable desktop hardware.
- Decoupled physics and render time steps.
- Smooth rendering of other riders.
- Robust reconnection.
- Offline playback of downloaded routes.
- Bounded memory consumption through world streaming.
- No dependency on a single GPU API.
- Consistent behavior on Linux, macOS and Windows.
- Automated tests on all three operating systems.

These values are development targets, not guaranteed performance figures.

---

## 16. Architecture Summary

| Component | Selected Technology |
|---|---|
| Programming language | Rust |
| 3D engine | Bevy |
| Rendering | wgpu |
| GUI | Bevy UI |
| Supplementary GUI | egui |
| Audio | Bevy Audio |
| Voice chat | WebRTC / LiveKit |
| Bluetooth | btleplug |
| ANT+ | Separate backend |
| Async | Tokio |
| Networking | QUIC / Quinn |
| API | Axum |
| Data format | Protobuf |
| Database | PostgreSQL |
| Cache | Redis (optional) |
| Geodata | OpenStreetMap PBF |
| Elevation model | Copernicus DEM / SRTM |
| 3D asset format | glTF 2.0 |
| Modeling | Blender |
| CI/CD | GitHub Actions |
| Deployment | Docker / Linux |

---

## Conclusion

Open Cycling World is conceived as an independent, Rust-based simulation platform.

The desktop client handles rendering, local world management, bicycle physics, device communication and user interaction.

The backend handles identity, multiplayer, synchronization, groups, races, voice rooms and data distribution.

The geographic world is generated from OpenStreetMap data and supplementary elevation data, stored locally, and rendered on the respective hardware.

The architecture is deliberately modular, so that individual components can later be replaced or extended without redeveloping the entire application.

**Primary development goal:** A technically independent, cross-platform indoor cycling system with real routes, real bicycle hardware, individual customization and synchronized multiplayer.
