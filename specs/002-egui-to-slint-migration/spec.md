# Feature Specification: Egui to Slint Migration

**Feature Branch**: `[002-egui-to-slint-migration]`  
**Created**: 2026-04-11  
**Status**: Draft  
**Input**: User description: "Draft the second spec for migration from egui/eframe to Slint, focused on desktop platforms (macOS, Windows, Linux)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Keep Existing Desktop Workflows While Replacing UI Framework (Priority: P1)

As a user, I need the app to preserve the current core workflows after migration so that changing UI framework does not break configuration, map usage, or stream playback.

**Why this priority**: Preserving existing workflows minimizes regression risk during a major framework change.

**Independent Test**: Can be tested by launching the Slint-based app and completing each existing workflow end-to-end without using egui.

**Acceptance Scenarios**:

1. **Given** the migrated app starts, **When** I open the configuration view, **Then** I can edit RTSP URL, ROV API base URL, and OSM User-Agent fields and trigger existing actions.
2. **Given** the migrated app starts, **When** I open the map view, **Then** I can detect location, adjust zoom, refresh map tiles, and open an interactive browser map.
3. **Given** the migrated app starts, **When** I open the stream view, **Then** I can start and stop embedded stream playback and see live frame updates and status.

---

### User Story 2 - Establish Desktop Portability Baseline with Slint (Priority: P2)

As a maintainer, I need the migrated UI to build and run on macOS, Windows, and Linux so that the project can evolve as a desktop cross-platform client.

**Why this priority**: Desktop portability is the strategic target and should be validated during migration, not deferred.

**Independent Test**: Can be tested by running platform-specific build and smoke-run checks for all three desktop targets.

**Acceptance Scenarios**:

1. **Given** the migrated codebase, **When** desktop build checks are run, **Then** the app builds successfully for macOS, Windows, and Linux configurations.
2. **Given** a desktop build artifact, **When** the app is launched, **Then** UI navigation and key workflows are functional on that platform.

---

### User Story 3 - Reduce Future Refactor Cost by Separating UI and Domain Logic (Priority: P3)

As a maintainer, I need UI-independent application logic boundaries so that future refactors do not require deep rewrites across map, stream, and API behavior.

**Why this priority**: Migration is the best point to reduce coupling that currently exists between egui rendering and app logic.

**Independent Test**: Can be tested by reviewing module boundaries and verifying that non-UI logic is callable without direct dependence on a specific UI framework.

**Acceptance Scenarios**:

1. **Given** the migrated codebase, **When** module boundaries are reviewed, **Then** UI rendering code is isolated from stream/map/API control logic.
2. **Given** a future UI change request, **When** implementation impact is assessed, **Then** changes are mostly limited to presentation-layer files.

---

### Edge Cases

- What happens when desktop-specific native behavior (such as macOS native location permission flow) differs from Windows or Linux behavior?
- How does the app behave when ffmpeg is missing or stream decoding fails during active rendering?
- What happens when map tile fetch fails due to network errors or policy-related request rejection?
- How does the UI handle rapid start/stop stream actions while worker threads are transitioning?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST replace egui/eframe-based UI rendering with Slint-based UI rendering for desktop builds.
- **FR-002**: The system MUST preserve existing user-visible workflows for configuration, map interaction, and live stream control.
- **FR-003**: The system MUST keep map tile loading, location detection, stream lifecycle control, and ROV API interactions functionally equivalent to current behavior unless explicitly documented otherwise.
- **FR-004**: The system MUST provide clear module boundaries between UI presentation and non-UI application logic.
- **FR-005**: The system MUST provide status/error messaging in the Slint UI for stream, map, and API actions at least equivalent in clarity to current behavior.
- **FR-006**: The system MUST define migration compatibility scope as macOS, Windows, and Linux; mobile platforms are out of scope for this migration.
- **FR-007**: The system MUST remove direct dependency on egui/eframe from runtime UI paths once migration is complete.
- **FR-008**: The system MUST maintain desktop packaging assumptions for bundled/external ffmpeg usage without introducing a hard runtime dependency regression.

### Key Entities *(include if feature involves data)*

- **UI Screen State**: The active view selection and UI-bound values that drive configuration, map, and stream interactions.
- **Application Logic Layer**: Non-rendering behavior for stream control, location detection, map tile fetching, and ROV API actions.
- **Stream Session**: The lifecycle state for ffmpeg process management, frame flow, and stream status events.
- **Map Session**: The location, zoom, and tile retrieval state needed to render and refresh map content.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of current core workflows (configuration actions, map actions, stream start/stop and rendering) are executable in the Slint-based app.
- **SC-002**: Desktop build validation passes for macOS, Windows, and Linux targets defined by the project.
- **SC-003**: No release-blocking regressions are identified in stream lifecycle handling, map refresh behavior, or ROV API actions during migration acceptance testing.
- **SC-004**: Post-migration code review confirms UI-framework-specific code is isolated to presentation modules, with non-UI logic reusable independently.

## Assumptions

- This migration is a framework replacement and does not aim to redesign product behavior or add new user-facing features.
- iOS is explicitly deferred and excluded from acceptance criteria for this specification.
- Android may be evaluated later as a separate effort and is not part of this migration scope.
- Existing network endpoints and stream/map behavior remain the baseline for compatibility during migration.
- Desktop platform validation may use staged checks (build first, then smoke-run) if full runtime parity automation is not yet available.
