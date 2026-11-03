# Test designs

Small designs for the engine's tests and for trying the tools by hand. They aren't examples: the engine has no examples (see docs/requirements.md).

| Folder | What it is |
| --- | --- |
| `blinky` | An LED blinking from a counter, for the Alhambra II (`main.v`, `main.pcf`) |
| `blinky-ecp5` | The same blinky for the iCESugar-Pro, an ECP5 board (`main.v`, `main.lpf`) |
| `blinky-gowin` | The same blinky for the Sipeed Tang Nano 9K, a Gowin board (`main.v`, `main.cst`) |
| `counter` | A 4-bit counter (`counter.v`) with a passing testbench (`counter_tb.v`) and a failing one (`counter_fail_tb.v`) |
| `syntax-error` | A module with a syntax error on line 3 (`main.v`) |

Tests copy a design into a temporary folder before using it, so these files are never changed.
