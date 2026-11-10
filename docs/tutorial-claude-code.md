# FPGA Design with Logix Engine and Claude Code

In this tutorial, [Claude Code](https://claude.com/claude-code) designs
hardware for you through Logix Engine. You describe what you want, such as
"make the LEDs sweep back and forth", and Claude writes the necessary source
files, builds them, tests them, shows the waveforms and programs the board.
You don't need to set up a toolchain, write Verilog yourself or even
understand the Verilog language.

> [!TIP]
> The prompts in this tutorial are in English, but you can write them in any
> language Claude understands, such as Spanish, German, Hebrew or Japanese.

## Background

Logix Engine ("the engine" in short) is an open source MCP server that takes
care of the FPGA design cycle: lint, build, test, simulate and program the
board. It works with AI agents such as Claude Code, and with regular apps
such as the [Logix IDE](https://www.openlogix.org/ide/). It downloads the
open source FPGA tools it needs, and knows about boards, with their FPGAs and
pins. Logix Engine is part of the [Open Logix](https://www.openlogix.org)
project.

## What you need

- An Apple Mac computer with Apple Silicon (M1 or later).
- Claude Code, signed in to a Claude account (step 1).
- An FPGA board from the engine's catalog, and its USB cable. The examples
  use the **Alhambra II**, which has 8 LEDs. Any board in the catalog works:
  replace "Alhambra II" with your board in the prompts, and Claude adapts
  the designs to it.

> [!NOTE]
> Without a board, you can still do every step but 10.

Each text block is labeled with where it goes:

**Terminal:**

```sh
echo "a command"
```

**Claude Code:**

```text
A prompt.
```

Type the prompts as they are, or in your own words or language.

## 1. Install Claude Code

Check whether Claude Code is installed:

**Terminal:**

```sh
claude
```

- **It starts:** Claude Code is installed. Type `/exit` and go to step 2.
- **"command not found", and you have Homebrew:** install it with Homebrew.
- **"command not found", and no Homebrew:** follow the
  [Claude Code install instructions](https://code.claude.com/docs/en/setup),
  then come back.

To install with Homebrew:

**Terminal:**

```sh
brew install --cask claude-code
```

Then run `claude`, sign in to your Claude account when it asks, and type
`/exit`.

## 2. Install the engine

Open the engine's [Releases](https://github.com/openlogix/logix-engine/releases)
page and download the newest `logix-engine-darwin-arm64-<date>.tgz` to your
**Downloads** folder.

Clear the download flag and unpack it:

**Terminal:**

```sh
cd ~/Downloads
xattr -c logix-engine-darwin-arm64-*.tgz
tar -xzf logix-engine-darwin-arm64-*.tgz
```

> [!IMPORTANT]
> The engine isn't signed with an Apple Developer ID yet. `xattr -c` tells
> macOS to allow it; without it, macOS refuses to run the engine.

Check that it runs:

**Terminal:**

```sh
~/Downloads/logix-engine/logix-engine --version
```

It prints the engine's version, a date such as `2026-11-09`.

## 3. Connect Claude Code to the engine

Register the engine with Claude Code once, for every folder:

**Terminal:**

```sh
ENGINE=~/Downloads/logix-engine/logix-engine
ENGINE_HOME=~/.logix-dev/engine
claude mcp add -s user logix-engine -- $ENGINE --engine-folder $ENGINE_HOME
claude mcp list
```

The list shows `logix-engine` as connected. The engine keeps the FPGA tools
it downloads, and its logs, in its engine folder, `~/.logix-dev/engine`.

## 4. Create the project

Make a folder for the tutorial's project. You do this only once:

**Terminal:**

```sh
mkdir -p ~/logix-tutorial
```

## 5. Start Claude Code

**Terminal:**

```sh
cd ~/logix-tutorial
claude
```

Ask what the engine can do:

**Claude Code:**

```text
What can the Logix engine do? Keep it short.
```

Claude lists the engine's tools: lint, build, test, simulate, program the
board and more.

> [!IMPORTANT]
> The first time Claude uses one of the engine's tools, Claude Code asks for
> permission: choose **"Yes, and don't ask again"** for the `logix-engine`
> tools, so the rest of the tutorial flows. Claude also asks before it
> writes files in your folder.

Now tell the engine to fetch the open source FPGA tools it needs:

**Claude Code:**

```text
Download the FPGA tools the engine needs, and show me the progress.
```

> [!NOTE]
> The tools are downloaded into the engine folder you set in step 3,
> `~/.logix-dev/engine`, and don't change your computer's setup. This takes
> a minute or two, only the first time.

## 6. Choose your board

**Claude Code:**

```text
Which boards does the engine support? Group them by FPGA family, names only.
```

The engine knows about 80 boards: their FPGA, how to program them, and
what's on their pins. Find yours, and ask about it:

**Claude Code:**

```text
Tell me about the Alhambra II: its FPGA, its clock, and its LEDs.
```

Claude answers from the engine's board catalog: an iCE40 HX4K, a 12 MHz
clock on pin 49, and 8 LEDs with their pins. It never has to guess a pin.
To see how two boards differ:

**Claude Code:**

```text
Compare the Alhambra II with the iCEstick.
```

## 7. Blink an LED

**Claude Code:**

```text
Start a new project here for the Alhambra II: blink LED 0 about once a second. Then build it.
```

Claude writes three files and builds them:

- `main.v`: the design, a counter whose top bit drives the LED.
- `pinout.pcf`: which FPGA pins the design's signals use.
- `logix.json`: Claude's notes on the project, such as its board.

The build ends with a bitstream in the `_build/` folder in the project, and
a short report: how much of the FPGA the design uses, and the fastest clock
it could run at.

## 8. Test it

On an FPGA, you test before you program the board. Ask for a testbench, a
simulation that checks what the design does:

**Claude Code:**

```text
Write a testbench that checks LED 0 turns on and off at the right times, using a small counter so the test is quick. Run it.
```

Claude writes `main_tb.v`, and the test passes. Now see a failure:

**Claude Code:**

```text
Change the testbench to expect the LED to turn on one clock cycle too early. Run it, explain the failure, then put it back.
```

The engine reports the failed check at its line and simulation time, and
Claude explains what went wrong.

## 9. See the signals

**Claude Code:**

```text
Simulate the testbench and open the waveforms in GTKWave.
```

GTKWave opens with the clock, the counter and the LED. Zoom in to watch the
counter count and the LED switch.

## 10. Program the board

Connect the board with its USB cable.

**Claude Code:**

```text
Is my board connected?
```

Claude lists what's connected, and how well each device matches your board.
Then:

**Claude Code:**

```text
Program the board with the design.
```

LED 0 blinks, about once a second: a design that went from a sentence to
real hardware.

## 11. Change it with prompts

From here on, say what you want, and Claude changes the design in your
project folder, tests it and programs the board. Try these one at a time,
and watch the board after each:

**Claude Code:**

```text
Change the design: make the 8 LEDs count in binary, about 4 counts a second. Update the testbench, run it, then program the board.
```

**Claude Code:**

```text
Change it into a Knight Rider light: one LED sweeping back and forth, about one sweep a second. Test it and program the board.
```

**Claude Code:**

```text
Make the sweeping LED leave a fading trail behind it, using PWM. Program the board.
```

**Claude Code:**

```text
Make LED 0 breathe slowly on and off, like a sleeping laptop, while the other LEDs keep sweeping.
```

Bigger changes work too:

**Claude Code:**

```text
Put the sweep in its own module with a speed parameter, give it its own testbench, and keep main.v short. Run all the tests.
```

> [!TIP]
> If the board does something you didn't expect, say what you see, such as
> "the LEDs are lit when they should be off", and let Claude find out why.

## 12. Ask questions

Claude knows your design and the engine's reports, so ask about either:

**Claude Code:**

```text
How much of the FPGA does the design use now? What uses the most?
```

**Claude Code:**

```text
What's the fastest clock this design could run at, and what limits it?
```

**Claude Code:**

```text
Explain main.v line by line, as if I'm new to Verilog.
```

**Claude Code:**

```text
Make a diagram of the design and open it.
```

**Claude Code:**

```text
Why did you choose that counter width? What would happen with a 20-bit counter?
```

## 13. When things go wrong

The engine reports problems as data, with their file and line, so Claude can
fix them:

**Claude Code:**

```text
Put a typo in main.v on purpose, build it, show me the error, then fix it.
```

**Claude Code:**

```text
Check the design for warnings with the linter, and fix them.
```

**Claude Code:**

```text
Format all the Verilog files.
```

Disconnect the board, then:

**Claude Code:**

```text
Program the board with the design.
```

Claude reports that the board isn't connected, and what to check. Connect it
again and ask again.

## 14. Keep going

Some ideas, each one prompt:

- "Show a progress bar on the LEDs that fills up every 3 seconds, then starts over."
- "Make a reaction game: light a random LED, and count how long until I press a button." Tell Claude which pin your board's button is on, if the engine's catalog doesn't list it.
- "Turn the LED pattern into a table at the top of main.v that I can edit."
- "Make the design work on the iCEstick too." Then ask what changed.

Start each new design in its own folder, and run `claude` there.

## Clean up

- Build files: ask Claude to clean the build folder, or delete `_build/`.
- To disconnect the engine from Claude Code:
  `claude mcp remove --scope user logix-engine`
- To remove the engine, delete `~/Downloads/logix-engine` and its engine
  folder, `~/.logix-dev/engine`.

## Learn more

- [Logix Engine](https://www.openlogix.org/engine/): what it is and where
  it's going.
- [Tool API](tool-api.md): every tool Claude used here, for writing your own
  frontend or agent.
- [Logix IDE](https://www.openlogix.org/ide/): the same engine in an app,
  the Arduino experience, for FPGAs.
- [Logix Catalog](https://www.openlogix.org/catalog/): the boards, and how
  to add yours.

Logix Engine is free and open source, under MIT or Apache 2.0. To report a
problem or share an idea, open an issue on
[GitHub](https://github.com/openlogix/logix-engine/issues).
