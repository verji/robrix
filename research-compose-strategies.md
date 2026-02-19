# Research: Strategies for Composing Verji Customizations with Robrix Community Work

**Date:** 2026-02-18
**Branch:** taj/compose-research
**Status:** Initial research complete

## Objective

Evaluate strategies for maintaining Verji-specific customizations to Robrix while continuing to benefit from upstream community development. The goal is to find a sustainable model that minimizes merge pain while enabling deep customization.

---

## Table of Contents

1. [Architecture Context](#1-architecture-context)
2. [Research Area: Plugin / Composable Component System](#2-research-area-plugin--composable-component-system)
3. [Research Area: Screen Real Estate and Layout Constraints](#3-research-area-screen-real-estate-and-layout-constraints)
4. [Research Area: Multi-Platform Complexity](#4-research-area-multi-platform-complexity)
5. [Research Area: Fork + Merge Upstream Strategy](#5-research-area-fork--merge-upstream-strategy)
6. [Research Area: Feature-Flag Composition Strategy](#6-research-area-feature-flag-composition-strategy)
7. [Comparative Evaluation](#7-comparative-evaluation)
8. [Recommendations](#8-recommendations)

---

## 1. Architecture Context

### How Robrix / Makepad Works Today

Robrix is a Matrix chat client built on the **Makepad** UI framework (Rust). Key architectural facts relevant to our evaluation:

| Aspect | How It Works |
|--------|-------------|
| **Widget system** | Structs with `#[derive(Live, Widget)]` + declarative `live_design!` DSL. All widget types resolved at **compile time**. |
| **Widget registration** | Each module has a `live_design(cx)` function called in dependency order during app startup. No runtime registry. |
| **Composition** | Prototypical inheritance in DSL: `MyWidget = <BaseWidget> { ...overrides... }`. Widget trees are static declarations. |
| **Namespace linking** | `cx.link(from_id, to_id)` redirects DSL lookups at registration time. Used for theme swapping and feature gates. |
| **Feature gating** | Cargo features + `#[cfg(feature = "...")]` + `cx.link()` for compile-time module substitution. Already proven with TSP wallet feature. |
| **Layout** | Flow-based (Down/Right/Overlay), no CSS. Dock widget provides splitter + tabs. AdaptiveView switches Desktop/Mobile layouts. |
| **Theming** | Color/font constants in `shared/styles.rs`. Theme override via `cx.link()`. No runtime theme switching. |
| **Data layer** | Matrix SDK operations isolated on background tokio thread. UI communicates via `crossbeam-channel` + `SignalToUI`. Clean separation. |
| **Platform support** | 7+ targets from single codebase (Windows/macOS/Linux/Android/iOS/Web). Desktop mature, mobile maturing. |
| **Rendering** | Custom GPU renderer with per-widget GLSL shaders. Not using native platform widgets. |

### Key Files for Customization

| File | Role | Modification Risk |
|------|------|-------------------|
| `app/src/app.rs` | App bootstrap, action dispatch, widget registration order | High - merge-sensitive |
| `app/src/shared/styles.rs` | All colors, fonts, icons (50+ constants) | Low - additive changes merge cleanly |
| `app/src/home/main_desktop_ui.rs` | Desktop Dock layout, tab management | Medium |
| `app/src/home/main_mobile_ui.rs` | Mobile stack layout | Medium |
| `app/src/sliding_sync.rs` | All Matrix SDK operations (~4000 LOC) | High - frequently changed upstream |
| `app/src/home/rooms_list.rs` | Room list rendering and filtering | Medium |
| `app/src/room/room_screen.rs` | Timeline rendering (~2200 LOC) | High - frequently changed upstream |
| `app/Cargo.toml` | Dependencies, features, build config | Medium - merge-sensitive |

---

## 2. Research Area: Plugin / Composable Component System

**Goal:** Determine if Makepad's architecture supports a plugin system where Verji-specific components can be swapped in without modifying upstream code.

### Findings

#### 2.1 Makepad Has No Plugin Architecture

Makepad's widget system is **entirely compile-time**:

- All widget types must be compiled into the binary. There is no dynamic loading, no trait-object-based widget dispatch, no runtime widget factory.
- The `live_design!` DSL compiles to static Rust data structures. Widget type names are resolved to concrete structs at build time.
- There is no plugin host, no WASM extension mechanism, no shared library loading.

**Conclusion:** A traditional plugin system (load extensions at runtime) is **not feasible** with current Makepad.

#### 2.2 What Makepad DOES Support: Compile-Time Composition

Makepad provides three mechanisms that approximate pluggability:

**Mechanism 1: Namespace Linking (`cx.link()`)**

Already proven in Robrix for the TSP feature:
```rust
// In app.rs LiveRegister implementation:
#[cfg(feature = "tsp")] {
    crate::tsp::live_design(cx);
    cx.link(id!(tsp_link), id!(tsp_enabled));
}
#[cfg(not(feature = "tsp"))] {
    crate::tsp_dummy::live_design(cx);
    cx.link(id!(tsp_link), id!(tsp_disabled));
}
```
Widgets in the DSL reference the `tsp_link` namespace. At compile time, the feature flag determines which implementation gets linked. The dummy module provides no-op widgets with identical interfaces.

**Applicability to Verji:** Could create a `verji` feature flag that links a `verji_link` namespace to Verji-specific widget implementations. Standard builds would link to default/community implementations.

**Limitation:** The namespace linking happens in `app.rs`, which is a merge-sensitive file. Every new feature-gated component requires touching this central registration point.

**Mechanism 2: DSL Prototypical Inheritance**

New widgets can extend existing ones without writing Rust code for a new backing struct:
```rust
live_design! {
    VerjiRoomHeader = <RoomHeader> {
        // Override colors, add branding
        draw_bg: { color: (#VerjiBlue) }
        company_logo = <Image> { source: dep("verji-logo.png") }
    }
}
```

**Applicability:** Good for theming and minor visual customizations. Cannot change behavior (event handling, business logic) - only layout, styling, and child widget composition.

**Mechanism 3: Cargo Feature Flags + Conditional Compilation**

Standard Rust feature flags can gate entire modules:
```toml
[features]
default = []
verji = ["dep:verji-sdk"]
```

**Applicability:** The most powerful mechanism. Can conditionally compile entirely different screen implementations, additional Matrix request handlers, custom authentication flows, etc.

#### 2.3 Could a Plugin System Be Built?

A compile-time "plugin" system could be architected using these mechanisms:

```
robrix (upstream)
├── app/src/
│   ├── app.rs              ← plugin registration point
│   ├── plugins/            ← plugin trait definitions
│   │   ├── mod.rs          ← PluginRegistry trait
│   │   ├── default/        ← default implementations
│   │   └── [feature-gated] ← custom implementations
│   ├── home/               ← uses plugin abstractions
│   └── room/               ← uses plugin abstractions
```

**However, this would require:**
1. Upstream Robrix to adopt a plugin architecture (trait-based abstractions at every extension point)
2. Agreement on plugin API stability
3. Significant refactoring of tightly-coupled components (e.g., `room_screen.rs` directly renders message bubbles)

**Assessment:** Getting upstream to adopt a plugin architecture for one downstream consumer is unlikely. This approach requires substantial community buy-in and adds complexity that the open-source project may not want.

#### 2.4 Comparison with Other Rust UI Frameworks

No major Rust UI framework has a formal plugin system:

| Framework | Component Model | Plugin Support |
|-----------|----------------|---------------|
| **Makepad** | DSL + compile-time registration | None (compile-time only) |
| **Dioxus** | React-like composition | None formal |
| **Slint** | DSL components + properties | None formal |
| **egui** | Pure Rust functions | None formal |

This is not a Makepad-specific limitation - it reflects the state of the Rust UI ecosystem.

### Verdict on Plugin System

| Criterion | Assessment |
|-----------|-----------|
| **Feasibility** | Not feasible as runtime plugins. Partially feasible as compile-time feature-gated modules. |
| **Upstream dependency** | Would require upstream architectural changes they're unlikely to make. |
| **Maintenance cost** | Even if built, plugin API stability would be a constant concern. |
| **Recommendation** | Do not pursue a traditional plugin architecture. Instead, use feature flags + fork strategies. |

---

## 3. Research Area: Screen Real Estate and Layout Constraints

**Goal:** Determine if a composable component system would impose layout restrictions (predefined dock areas, static regions, etc.).

### Findings

#### 3.1 Makepad Layout System Is Flexible

Makepad uses a **flow-based layout** (not CSS grid/flexbox):

- **Flow directions:** `Down` (vertical), `Right` (horizontal), `Overlay` (stacked)
- **Size constraints:** `Fill` (expand), `Fit` (shrink-to-content), `Fixed(px)`
- **No predefined dock areas:** Layout is entirely defined in the `live_design!` DSL

The existing Robrix layout demonstrates this flexibility:

**Desktop Layout:**
```
┌──────────┬──────────────────────────────────┐
│ NavBar   │ Dock (Splitter)                   │
│ (fixed)  │ ┌──────────┬─────────────────────┤
│          │ │ Rooms    │ Room Content         │
│          │ │ (300px)  │ (Fill)               │
│          │ │          │ ┌─────────────────┐  │
│          │ │          │ │ Timeline        │  │
│          │ │          │ │ (PortalList)    │  │
│          │ │          │ ├─────────────────┤  │
│          │ │          │ │ Message Input   │  │
│          │ │          │ └─────────────────┘  │
│          │ └──────────┴─────────────────────┘│
└──────────┴──────────────────────────────────┘
```

**Mobile Layout:**
```
┌──────────────────┐
│ Content (Fill)   │
│ (Stack-based)    │
│                  │
│                  │
├──────────────────┤
│ NavBar (bottom)  │
└──────────────────┘
```

#### 3.2 Dock Widget Provides Dynamic Panels

The Dock widget (`main_desktop_ui.rs`) supports:
- **Splitter:** Draggable divider between two panes (horizontal or vertical)
- **Tabs:** Multiple content views in tabbed interface (closeable tabs for rooms)
- **State persistence:** Layout saved/restored via `SavedDockState` (serialized JSON)

Adding new panels or rearranging the dock layout requires modifying the DSL definition, but does **not** require predefined static dock areas. The layout is fully programmable.

#### 3.3 AdaptiveView Handles Responsive Layouts

The `AdaptiveView` widget selects between layout variants based on window size:
```rust
// Variant selector callback
|_cx, parent_size| {
    match parent_size.x {
        width if width <= 70.0  => id!(OnlyIcon),
        width if width <= 200.0 => id!(IconAndName),
        _ => id!(FullPreview),
    }
}
```

Custom Verji components could leverage this same mechanism for responsive behavior.

#### 3.4 CachedWidget Preserves State Across Layout Changes

When switching between Desktop and Mobile layouts, `CachedWidget` ensures singleton widgets maintain their state (scroll position, selection, etc.). This pattern would work for Verji-specific widgets too.

### Verdict on Layout Constraints

| Criterion | Assessment |
|-----------|-----------|
| **Static dock areas required?** | **No.** Layout is fully defined in DSL, no predefined regions. |
| **Can Verji add panels/regions?** | **Yes.** Modify DSL to add new Dock tabs, splitter panes, or overlay panels. |
| **Responsive support?** | **Yes.** AdaptiveView + CachedWidget pattern handles layout switching. |
| **Constraint** | Layout changes touch DSL files that upstream also modifies (merge conflict risk). |

---

## 4. Research Area: Multi-Platform Complexity

**Goal:** Assess how cross-platform support affects customization complexity.

### Findings

#### 4.1 Makepad's Cross-Platform Model

Makepad renders everything through its **own GPU renderer** - it does NOT use native platform widgets. This means:

| Platform | Rendering Backend | Maturity |
|----------|------------------|----------|
| Windows | DirectX 11 | Mature |
| macOS | Metal | Mature |
| Linux | OpenGL | Mature |
| Android | OpenGL ES | Functional, maturing |
| iOS | Metal | Functional, maturing |
| Web/WASM | WebGL | Requires nightly Rust |

**Key insight:** Because Makepad does its own rendering, custom widgets look and behave identically across all platforms. There is no platform-specific widget styling or behavior to worry about.

#### 4.2 What IS Platform-Specific

Platform differences are handled at two levels:

1. **Layout adaptation (handled by Robrix):**
   - `AdaptiveView` switches Desktop vs Mobile layout based on window size
   - Both layouts share identical widget implementations and business logic
   - Adding a Verji customization to a widget works on ALL platforms automatically

2. **OS integration (handled by Robius crates):**
   - `robius-open` - Opening URLs/files
   - `robius-directories` - Platform-appropriate file paths
   - `robius-location` - GPS/location services
   - These are abstracted behind cross-platform APIs

#### 4.3 Complexity Impact Assessment

| Customization Type | Multi-Platform Impact |
|---|---|
| **Theming (colors, fonts, icons)** | Zero - renders identically everywhere |
| **New widgets (UI components)** | Zero - Makepad renderer is cross-platform |
| **Layout changes (panels, tabs)** | Low - may need Desktop AND Mobile variants in AdaptiveView |
| **OS integration (notifications, file access)** | Medium - need to use/extend Robius abstractions |
| **Native embedding (webview, native controls)** | High - platform-specific code required per target |

### Verdict on Multi-Platform Complexity

| Criterion | Assessment |
|-----------|-----------|
| **Does multi-platform add customization complexity?** | **Minimal** for UI changes. Makepad's own renderer means widgets work everywhere. |
| **Key exception** | If Verji needs native platform features (webview, system notifications), complexity increases per platform. |
| **Recommendation** | Design Verji customizations as pure Makepad widgets wherever possible to avoid platform-specific code. |

---

## 5. Research Area: Fork + Merge Upstream Strategy

**Goal:** Evaluate maintaining a Verji fork of Robrix with regular upstream merges.

### 5.1 How This Would Work

```
upstream/robrix (main)
    │
    ├── v0.1 ──── v0.2 ──── v0.3 ──── v0.4 ────►
    │              │                    │
    │              │ merge              │ merge
    │              ▼                    ▼
    └── verji/robrix (fork)
        ├── verji-branding ─── verji-auth ─── new-upstream ─── ...
```

### 5.2 Merge Conflict Risk by Area

Based on analysis of the codebase structure:

| Area | Files | Upstream Change Frequency | Verji Change Likelihood | Conflict Risk |
|------|-------|--------------------------|------------------------|---------------|
| **Theming** | `shared/styles.rs` | Low (additive) | High | **Low** - different constants |
| **App bootstrap** | `app.rs` | Medium | High (registration) | **High** - same code region |
| **Room screen** | `room_screen.rs` | High (active development) | Medium | **High** - large, active file |
| **Rooms list** | `rooms_list.rs` | High | Low-Medium | **Medium** |
| **Sliding sync** | `sliding_sync.rs` | High (SDK updates) | Low | **Low** - Verji unlikely to change |
| **Desktop layout** | `main_desktop_ui.rs` | Medium | High | **High** - DSL changes |
| **Settings** | `settings_screen.rs` | Low | High | **Low** - Verji adds sections |
| **Cargo.toml** | `Cargo.toml` | High (dep updates) | High (new deps) | **Medium** |
| **New Verji modules** | `verji/` subdirectory | None (doesn't exist upstream) | High | **Zero** - no upstream overlap |

### 5.3 Strategies to Minimize Merge Pain

**Strategy A: Verji Code in Separate Directory**

Place all Verji-specific code in `app/src/verji/` with its own module tree:
```
app/src/
├── verji/                    ← All Verji code here (zero upstream overlap)
│   ├── mod.rs
│   ├── branding.rs           ← Verji theming overrides
│   ├── auth.rs               ← Custom authentication
│   ├── settings_panel.rs     ← Verji settings section
│   └── widgets/              ← Custom widgets
├── app.rs                    ← Minimal touch: add verji::live_design(cx)
├── shared/styles.rs          ← Minimal touch: add Verji color constants
└── ...                       ← Upstream code untouched
```

**Merge impact:** Only `app.rs` (one line: `verji::live_design(cx)`) and `Cargo.toml` (feature flag) need changes in upstream files. Everything else is additive.

**Strategy B: Feature-Flag Gated Integration Points**

Combine with Cargo feature flags:
```toml
[features]
default = []
verji = []
```

```rust
// In app.rs - single merge-sensitive line
#[cfg(feature = "verji")]
crate::verji::live_design(cx);
```

**Merge impact:** Even lower - the feature flag code is a single `#[cfg]` block that rarely conflicts.

**Strategy C: Upstream Contribution of Extension Points**

Contribute back to upstream generic extension points that Verji needs:
- Settings screen extensibility (callback/hook for adding settings panels)
- Theme override system improvements
- Login flow customization hooks

If upstream accepts these, Verji's fork divergence decreases over time.

### 5.4 Known Risks

| Risk | Severity | Mitigation |
|------|----------|-----------|
| **Makepad breaking changes** | High | Robrix tracks a specific Makepad branch; breakage is gradual |
| **Matrix SDK API changes** | High | Upstream handles this; Verji inherits fixes via merge |
| **Architectural refactoring** | Medium | If upstream restructures modules, Verji integration points may break |
| **DSL syntax changes** | Low | Makepad DSL is relatively stable |
| **Build system changes** | Medium | Cargo.toml conflicts are mechanical (not semantic) |

### Verdict on Fork + Merge

| Criterion | Assessment |
|-----------|-----------|
| **Feasibility** | **High.** Well-understood approach. Rust's module system and feature flags help isolate changes. |
| **Maintenance cost** | **Moderate.** Regular merges needed (~monthly?). Cost depends on discipline of isolating Verji code. |
| **Risk of divergence** | **Manageable** if Verji code stays in dedicated directory with minimal upstream file changes. |
| **Community contribution** | **Possible.** Can contribute non-Verji improvements back upstream. |

---

## 6. Research Area: Feature-Flag Composition Strategy

**Goal:** Evaluate a deeper integration where Verji customizations are managed through Cargo feature flags, potentially even contributed upstream.

### 6.1 How This Works (Proven Pattern)

Robrix already does this with the TSP feature:

```rust
// Cargo.toml
[features]
tsp = ["dep:tsp-sdk", "dep:tsp-definitions"]

// app.rs
#[cfg(feature = "tsp")] {
    crate::tsp::live_design(cx);
    cx.link(id!(tsp_link), id!(tsp_enabled));
}
#[cfg(not(feature = "tsp"))] {
    crate::tsp_dummy::live_design(cx);
    cx.link(id!(tsp_link), id!(tsp_disabled));
}
```

The TSP module provides:
- Full TSP settings screen
- TSP verification modal
- TSP wallet management
- All gated behind a single feature flag

When disabled, dummy widgets provide no-op implementations with identical DSL interfaces.

### 6.2 Applied to Verji

```rust
// Cargo.toml
[features]
verji = ["dep:verji-auth-sdk"]

// app.rs
#[cfg(feature = "verji")] {
    crate::verji::live_design(cx);
    cx.link(id!(verji_link), id!(verji_enabled));
    cx.link(id!(login_variant), id!(verji_login));
    cx.link(id!(theme), id!(theme_verji));
}
```

### 6.3 What Can Be Feature-Gated

| Customization | Implementation | Complexity |
|---|---|---|
| **Branding / theming** | Override theme via `cx.link()` | Low |
| **Custom login flow** | Feature-gated login module with `cx.link()` substitution | Medium |
| **Additional settings panels** | New settings module, registered conditionally | Low |
| **Custom message rendering** | Feature-gated message widget override | High (merge-sensitive) |
| **Additional navigation tabs** | Modify home screen DSL (merge-sensitive) | Medium |
| **Custom Matrix request handlers** | Extend `MatrixRequest` enum (merge-sensitive) | High |

### 6.4 Limitations

- **Upstream must accept the feature flag** if not forked (or maintain in fork)
- **Cannot change upstream widget behavior** without modifying upstream code
- **Enum extensions** (e.g., `MatrixRequest`) don't compose well across feature flags
- **DSL namespaces** must be coordinated (can't have two independent features trying to override the same namespace)

### Verdict on Feature-Flag Strategy

| Criterion | Assessment |
|-----------|-----------|
| **Feasibility** | **High.** Proven pattern in Robrix (TSP). |
| **Best for** | Additive features, branding, alternative implementations of self-contained screens. |
| **Not suitable for** | Deep modifications to core screens (timeline, room list), cross-cutting concerns. |
| **Combines with** | Fork strategy - use feature flags WITHIN a fork for clean isolation. |

---

## 7. Comparative Evaluation

### Strategy Comparison Matrix

| Criterion | Plugin System | Fork + Merge | Feature Flags | Feature Flags in Fork |
|-----------|:---:|:---:|:---:|:---:|
| **Feasibility** | Low | High | High | **High** |
| **Upstream dependency** | Requires upstream changes | None | Requires upstream acceptance | **None** |
| **Merge conflict risk** | N/A | Medium | Low (if upstream) | **Low** |
| **Customization depth** | High (if built) | Unlimited | Medium (additive only) | **High** |
| **Maintenance cost** | Very high | Medium | Low | **Low-Medium** |
| **Community benefit** | High (if adopted) | Low | Medium | **Medium** |
| **Multi-platform impact** | N/A | None | None | **None** |
| **Time to implement** | Months+ | Days | Days-Weeks | **Days-Weeks** |

### Risk Matrix

| Risk | Plugin | Fork | Feature Flags | FF in Fork |
|------|:---:|:---:|:---:|:---:|
| **Upstream breaking changes** | High | Medium | Low | Medium |
| **Divergence over time** | Low | High | Low | **Medium** |
| **Developer onboarding complexity** | High | Low | Low | **Low** |
| **Build complexity** | High | Low | Low | **Low** |

---

## 8. Recommendations

### Recommended Strategy: Feature Flags Within a Maintained Fork

**Combine the fork approach with feature-flag isolation.** This gives the benefits of both:

1. **Fork Robrix** to `verji/robrix` (or internal repo)
2. **Isolate all Verji code** in `app/src/verji/` directory
3. **Gate with feature flag** `verji` in Cargo.toml
4. **Minimize upstream file changes** to:
   - `app.rs`: One `#[cfg(feature = "verji")]` block for registration
   - `Cargo.toml`: Feature flag + optional dependencies
   - `shared/styles.rs`: Verji color constants (additive)
5. **Merge upstream regularly** (monthly or per-release)
6. **Contribute non-Verji improvements** back upstream

### Implementation Phases

**Phase 1: Foundation (Low risk)**
- Fork and set up CI for both `default` and `verji` feature builds
- Create `app/src/verji/` module with branding/theme overrides
- Validate upstream merge workflow

**Phase 2: Customization (Medium risk)**
- Implement Verji-specific login flow (if needed)
- Add Verji settings panel
- Custom branding via `cx.link()` theme override

**Phase 3: Deeper Integration (Higher risk)**
- Custom message types or rendering (if needed)
- Additional Matrix request handlers
- Platform-specific integrations (notifications, etc.)

### What NOT to Do

- **Do not attempt a runtime plugin system.** Makepad does not support it and building one would be a multi-month project with uncertain upstream acceptance.
- **Do not modify upstream files extensively.** Every line changed in an upstream file is a potential merge conflict. Prefer additive code in the `verji/` directory.
- **Do not maintain a completely divergent fork.** Without regular merges, the fork will drift and become unmergeable within months.
- **Do not add platform-specific code unless absolutely necessary.** Makepad's cross-platform renderer means pure-Makepad widgets work everywhere.

---

## Appendix A: Makepad Architecture Details

### Widget Registration Flow
```
app_main!(App)
  -> LiveRegister::live_register(cx)
    -> makepad_widgets::live_design(cx)          // Built-in widgets
    -> cx.link(id!(theme), id!(theme_desktop_light))  // Theme
    -> shared::live_design(cx)                   // Shared components
    -> [feature-gated modules]                   // TSP, Verji, etc.
    -> home::live_design(cx)                     // Screens
    -> room::live_design(cx)
    -> settings::live_design(cx)
    -> ...
```

### Namespace Linking Mechanism
```rust
// DSL references a namespace:
MyWidget = <verji_link::CustomHeader> { ... }

// At registration, cx.link() resolves the namespace:
cx.link(id!(verji_link), id!(verji_enabled));
// Now verji_link::CustomHeader -> verji_enabled::CustomHeader
```

### Feature Flag Pattern (TSP Precedent)
```
app/src/
├── tsp/                    # Full implementation (feature = "tsp")
│   ├── mod.rs
│   ├── tsp_settings.rs
│   └── tsp_verification.rs
├── tsp_dummy/              # No-op stubs (feature != "tsp")
│   └── mod.rs
```

### Layout System Summary
- **Flow:** Down | Right | Overlay
- **Sizing:** Fill | Fit | Fixed(px)
- **Containers:** View (basic), Dock (splitter+tabs), PortalList (virtual scroll), PageFlip (multi-page), Modal (overlay), AdaptiveView (responsive)
- **No CSS, no grid, no flexbox** - simpler but sufficient for chat UI

## Appendix B: Key Observations from Codebase Analysis

1. **`sliding_sync.rs` (~4000 LOC)** is the nerve center for all Matrix operations. It's frequently changed upstream but Verji is unlikely to need to modify it directly (Matrix protocol operations are standard).

2. **`room_screen.rs` (~2200 LOC)** is the most complex UI file. If Verji needs custom message rendering, this file would be the main conflict zone. Consider whether Verji-specific message types can be handled as additions rather than modifications.

3. **`app.rs` (~770 LOC)** is the action dispatch hub. The `handle_actions()` method is a large match/if-else chain. Verji additions here should be wrapped in `#[cfg(feature = "verji")]` blocks at the end of the method to minimize merge conflicts.

4. **The `AdaptiveView` pattern** means any Verji widget added to the home screen needs both Desktop and Mobile variants. This doubles the layout work but not the logic.

5. **`CachedWidget`** is essential for any widget that needs to preserve state across Desktop<->Mobile switching. Verji widgets in navigation areas should use this pattern.

6. **No runtime theme switching** - Makepad resolves themes at startup. If Verji needs light/dark mode switching, this requires Makepad-level changes (not just Robrix).
