// Blinks LED0 at about 1.4 Hz from the 12 MHz clock.
module main (
    input  CLK,
    output LED0
);
  reg [22:0] count = 0;

  always @(posedge CLK) count <= count + 1;

  assign LED0 = count[22];
endmodule
