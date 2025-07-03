# Contributing to Hyperion

Welcome to the Hyperion project! We love pull requests from the community. Contributors who consistently provide quality PRs may be eligible for sponsored subscriptions to AI development tools from @andrewgazelka to enhance their productivity.

## Table of Contents
- [Development Tools](#development-tools)
- [Protocol Documentation](#protocol-documentation)
- [Working with Protocol Documentation and LLMs](#working-with-protocol-documentation-and-llms)
- [Getting Help](#getting-help)
- [Development Setup](#development-setup)

## Development Tools

Essential tools to enhance your development workflow:

### Protocol Analysis
- **[Packet Inspector](https://github.com/valence-rs/valence/tree/main/tools/packet_inspector)**
  - Debug and analyze Minecraft protocol packets
  - Essential for protocol-related development

### Profiling
- **[Tracy Profiler](https://github.com/wolfpld/tracy)**
  - Advanced performance profiling
  - Helps identify bottlenecks

### Asset Management
- **[Mineskin](https://mineskin.org/)**
  - Upload skin PNG files
  - Get Mojang-signed signatures for player skin customization
 
You can use https://github.com/andrewgazelka/mineskin-cli to use Mineskin from CLI.

https://docs.mineskin.org/docs/guides/getting-started/


## Protocol Documentation

We currently target Minecraft 1.20.1 protocol specification:

- **Primary Reference**: [Wiki.vg Protocol (Version 18375)](https://wiki.vg/index.php?title=Protocol&oldid=18375)

## Working with Protocol Documentation and LLMs

Follow these steps to effectively use LLMs for packet analysis:

### 1. Obtaining Wiki.vg Documentation
1. Install the [MarkDownload](https://chromewebstore.google.com/detail/markdownload-markdown-web/pcmpcfapbekmbjjkdalcgopdkipoggdi) browser extension
2. Navigate to [Wiki.vg Protocol](https://wiki.vg/index.php?title=Protocol&oldid=18375)
3. Use MarkDownload to save the page as Markdown
4. Use the resulting file for LLM analysis

### 2. Valence Integration
1. Reference Valence's [packets.json](https://github.com/valence-rs/valence/blob/8f3f84d557dacddd7faddb2ad724185ecee2e482/tools/packet_inspector/extracted/packets.json)
2. This helps map Wiki.vg specifications to Valence implementations

### 3. LLM Interaction
- Provide both Wiki.vg markdown and relevant packets.json sections
- Get assistance with:
  - Packet structure mapping
  - Implementation details
  - Field mapping between specifications

## Getting Help

Multiple channels are available for support:

- **Issues**: Open a GitHub issue for detailed discussions
- **Discord**: Join our [community](https://discord.gg/PBfDtj5Wb) for real-time help
- **Existing Resources**: Check open PRs and issues for similar topics

## Development Setup

### Recommended IDEs

#### Primary IDE
- **IntelliJ IDEA**
  - Recommended as the primary development environment

#### AI-Enhanced Development
- **[Cursor](https://cursor.com)**
  - AI-powered coding assistance
  - Enhanced with [cursor-sync](https://github.com/andrewgazelka/cursor-sync) for IntelliJ position synchronization

#### Additional Tools
- **[Supermaven](https://www.supermaven.com)**
  - Enhanced code completion capabilities

### Code Quality Enforcement

We use pre-commit hooks to maintain code quality:

1. **Installation**
   ```bash
   pip install pre-commit
   ```

2. **Setup** (Optional)
   ```bash
   pre-commit install
   ```

3. **Manual Run** (Optional)
   ```bash
   pre-commit run --all-files
   ```

#### Automated Checks
The pre-commit configuration handles:
- Rust code formatting via `rustfmt`
- Additional code quality verifications

Hooks run automatically on `git commit`. If checks fail:
1. Review the reported issues
2. Make necessary corrections
3. Attempt the commit again
