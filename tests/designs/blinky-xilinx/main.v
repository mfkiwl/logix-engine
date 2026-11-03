// Blinks LED0 at about 3 Hz from the 100 MHz clock.
module main (
    input  CLK,
    output LED0
);
  reg [24:0] count = 0;

  always @(posedge CLK) count <= count + 1;

  assign LED0 = count[24];
endmodule
