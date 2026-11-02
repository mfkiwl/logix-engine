// A 4-bit counter with a synchronous reset.
module counter (
    input            clk,
    input            reset,
    output reg [3:0] q
);
  always @(posedge clk)
    if (reset) q <= 0;
    else q <= q + 1;
endmodule
