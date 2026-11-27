# Coding guidelines

These govern every file in `software/`.

- **The library serves its applications.** Change any API freely: no legacy, migration or compatibility code. When something is hard to do from an application (a workaround, an unsafe pin grab, a copied constant, a missing API), change Omnimotion instead, and move generic code an application had to write into it.

- **Readable first.** Plain, obvious code over clever code. If a reader has to reconstruct an argument to follow a function, the function is wrong.
- **Minimalist.** Nothing redundant, nothing extravagant. No abstraction without a second caller, no configuration knob nobody asked for, no speculative generality.
- **Files stay well under 1000 lines.** In practice most land between 100 and 300. A file growing past that is a signal to split by concern, not to keep appending.
- **No long comment blocks.** No banner headers, no narration of what the next line does, no restating the function name in prose. A comment earns its place only by recording a constraint the code cannot express, such as a datasheet requirement, a hardware erratum, or a timing rule. The StallGuard settle time is a good comment; `// set the current` is not.
- **Prefer good libraries.** Reach for an existing crate when it is lightweight, fast, open source and maintained. Write it ourselves when the crate is stale, heavier than the problem, or would distort the design. Record every such decision in the README dependency list with its reason.
- **Foundation before features.** Each phase leaves the workspace compiling and tested.
- **Test what is testable on the host.** Logic that can be pure stays pure and gets unit tests. Hardware glue stays thin enough to read and verify by inspection.

## Layout rule

`software/` is the host-testable workspace and must never depend on `esp-hal`. Anything touching Espressif hardware belongs in `software/firmware/`, which is a separate workspace on the Xtensa toolchain.

When a feature has both a decision and a hardware side, the decision goes in `omnimotion` and the board crate keeps a module of the same name that only reads pins and calls it, as `check`, `persistence` and `ota` do. Both crates group modules by concern (motion, storage or machine state, devices, network or update); a new module joins an existing group before it starts a new one.
