# Boards

One file per board, `<id>.json`, in the format in [docs/model.md](../docs/model.md). The data is free to use, copy and change, under MIT or Apache 2.0.

**Using it in a tool:** the intended way for tools, scripts and agents to get board data is [Logix Engine](https://www.openlogix.org/engine/)'s API (`get_catalog`). It's the supported interface: it serves the catalog version the engine builds and programs with, reads your custom boards the same way, and adds what the engine derives from the data. The layout of these files may change between releases, so tools that read them directly may need updating. This isn't enforced: you can read the files directly, at your own risk.

**Adding or fixing a board:** edit its file here and open a pull request; see [Adding or fixing a board](../README.md#adding-or-fixing-a-board). Your own boards use the same format, in your project's boards folder.
