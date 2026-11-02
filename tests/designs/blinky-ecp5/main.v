// Blinks LED0 at about 1.5 Hz from the 25 MHz clock.
module main (
    input  CLK,
    output LED0
);
  reg [23:0] count = 0;

  always @(posedge CLK) count <= count + 1;

  assign LED0 = count[23];
endmodule
