# Hyperion Minecraft Engine

## Overview

Hyperion is a custom Minecraft game engine designed for building scalable, high-performance game modes and events.
Unlike traditional Minecraft server implementations, Hyperion takes a ground-up approach to game mechanics through its
plugin-first architecture.

## Key Features

### Plugin-First Architecture

Hyperion starts with minimal base features and implements game mechanics through a fundamental plugin system. This
approach differs from traditional Minecraft servers that modify vanilla implementations:

- Core mechanics like combat are implemented as plugins
- Easily swap between different combat systems (e.g., 1.8 vs modern combat)
- Flexible customization without complex patching of vanilla code

### Entity Component System (ECS)

Hyperion utilizes [Bevy ECS](https://bevy.org/), an Entity Component System, as its core architecture:

- Entities are organized in a table-like structure
    - Rows represent individual entities
    - Columns represent components (e.g., health, position)
- Systems process entities through efficient iterations
- Components can be dynamically added (e.g., LastAttacked component for combat)
- For a more accurate representation of an ECS, see the [Unofficial Bevy Cheat Book](https://bevy-cheatbook.github.io/programming/ecs-intro.html)

### Performance Optimization

#### Parallel Processing

The ECS architecture enables efficient parallel processing:

- Entities are automatically partitioned across available threads
- Systems can process multiple entities simultaneously
- Automatic handling of dependencies and thread safety
- Optimal resource utilization while maintaining data consistency

:green{hola} oi

#### Proxy Layer

Performance bottlenecks are addressed through a sophisticated proxy system:

- Horizontally scaled proxy layer
- Vertically scaled game server
- Efficient packet broadcasting:
    - Global broadcast capabilities
    - Regional broadcasting for proximity-based updates
    - Optimized movement packet distribution

### Scalability

Hyperion is designed to handle large-scale events efficiently:

- Support for up > 10,000 concurrent players
- Performance constraints:
    - 20 ticks per second (50ms per tick)
    - Optimized processing within timing constraints
- Visibility optimization:
    - Configurable player render limits (400-700 players)
    - Customizable nametag visibility
    - Moderator-specific viewing options

## Technical Considerations

### Performance Management

- FPS optimization through selective rendering
- Nametag rendering management for performance
- Regional packet distribution to reduce network load
- Modular performance settings for different user roles

### Resource Utilization

- 50ms processing window per tick
- Balanced distribution of computational resources
- Efficient handling of IO operations through proxy layer
- Optimized packet management for large player counts

## Use Cases

Hyperion is ideal for creating custom Minecraft experiences similar to popular servers like Hypixel or Mineplex, where
vanilla mechanics can be completely customized to create unique game modes and events.

## Getting Started

Developers interested in using Hyperion should familiarize themselves with:

- Entity Component Systems (particularly Bevy)
- Minecraft networking protocols
- Parallel processing concepts
- Plugin development principles
