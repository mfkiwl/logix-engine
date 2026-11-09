# FPGAs

The FPGAs Logix supports, one file per FPGA family (`<family>.json`), in the format in [docs/model.md](../docs/model.md). They list every FPGA the toolchains support, so boards never add FPGAs. The data is free to use, copy and change, under MIT or Apache 2.0.

The intended way for tools, scripts and agents to get FPGA data is [Logix Engine](https://www.openlogix.org/engine/)'s API (`get_catalog`), the supported interface. The layout of these files may change between releases. This isn't enforced: you can read the files directly, at your own risk.
