`timescale 1ns / 1ps
// Fails on purpose: expects 8 where the counter is at 9.
module counter_fail_tb;
  reg clk = 0, reset = 1;
  wire [3:0] q;

  counter dut (
      .clk(clk),
      .reset(reset),
      .q(q)
  );

  always #5 clk = ~clk;

  initial begin
    $dumpvars(0, counter_fail_tb);
    #10 reset = 0;
    #90;
    if (q !== 4'd8) $error("q is %0d, expected 8", q);
    $finish;
  end
endmodule
