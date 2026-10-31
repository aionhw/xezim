# TileLink-UL subset: contract for the DVCon isolated showcase

**Protocol family:** TileLink Uncached Lightweight (TL-UL), A request and D response channels. This is a **narrow, disclosed 32-bit single-beat profile**, not TL-UH bursts or coherent TL-C (B/C/E channels). It follows the A/D opcode, source, mask, valid/ready and response concepts in the lowRISC OpenTitan TileLink-UL documentation and protocol checker.

## Required semantics

- One clock, synchronous active-low startup reset. No reset while a request is in flight in this initial profile. An in-flight-reset extension requires a separately specified and independently checked cancellation policy.
- Request accepted only on `a_valid && a_ready` at the rising clock edge. Hold A opcode, source, address, mask, size and data stable while stalled. `a_ready` may backpressure arbitrarily.
- One 32-bit data beat per request. `a_size = 2`, naturally aligned addresses, within a 1,024-byte device window.
- A opcodes: `PutFullData = 0`, `PutPartialData = 1`, `Get = 4`. D opcodes: `AccessAck = 0` for puts; `AccessAckData = 1` for gets. Unsupported opcodes are outside the qualified input profile.
- `PutFullData` must have mask `0b1111`. `PutPartialData` may use a **nonempty, not necessarily contiguous** four-lane mask. `Get` has mask `0b1111`. Each bit maps directly to the corresponding byte of `a_data`.
- This synthetic memory endpoint applies accepted writes at their A handshake. Reads snapshot the resulting memory value at their A handshake; it does not promise a separate global memory ordering model or multi-beat cache line behavior. This explicit endpoint policy must be supplied to all modes; it is NOT silently represented as a general TileLink guarantee.
- `a_source` is an outstanding transaction identifier, not a physical core ID. An identifier may not be reissued while pending. It can be reused after the matching D response is **accepted**. Each D response echoes the request's source and size and uses the correct opcode. Distinct sources may return out of order.
- D source/opcode/size/data/error remain stable while `d_valid && !d_ready`. A D handshake retires exactly one pending request. All requests drain at end of clean schedule; unexpected/lost/orphan responses are errors.
- This profile's qualified inputs do not exercise `d_denied`, `d_corrupt`, integrity parity, user fields, atomics, coherent probes, or multi-beat transfers; results do not imply support for those features.

## Why this is diagnostically interesting

The 16-source synthetic workload includes backpressure, independent bank completion delays, cross-source reorder, source reuse, partial writes, zero-data writes, and later dependent reads. Faults in the endpoint can be confused with a driver-intent mismatch or a response correlation error; the independent checker examines **accepted pin transfers** and does not trust any UVM scoreboard diagnosis.

The experiment must separately demonstrate that the clean run actually has >1 concurrent transaction, a D stall, at least one partial write, and cross-source completion inversion. If any witness is absent, P1 FAILS. An agent-only software log is not a substitute.

## Wire fields and record contract

`a_valid`, `a_ready`, `a_opcode[2:0]`, `a_size[2:0]`, `a_source[3:0]`, `a_address[15:0]`, `a_mask[3:0]`, `a_data[31:0]`, `d_valid`, `d_ready`, `d_opcode`, `d_size`, `d_source[3:0]`, `d_data[31:0]`, `d_error`.

The TB prints `PIN|cycle=...|av=...|ar=...|...` at the rising clock edge. The checker computes its own reference memory from accepted A handshakes, matches D by outstanding source, checks D stability under backpressure, and requires complete draining. It also reports traffic witnesses. No simulator-internal writer/source or hidden mutation locator is read by the checker.

## Authoritative references

- OpenTitan TL-UL bus: https://opentitan.org/book/hw/ip/tlul/index.html
- OpenTitan TileLink-UL protocol checker: https://opentitan.org/book/hw/ip/tlul/doc/TlulProtocolChecker.html
- The broader TileLink specification defines additional coherent and multi-beat behavior which is **not** claimed by this profile.
