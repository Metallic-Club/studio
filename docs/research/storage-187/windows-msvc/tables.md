# Storage probe evidence

Requested live heap includes retained capacities, not allocator headers or process RSS. World rows include services, Model, slots, children, free list, saturated journal and the ID vector. Component rows include only override containers and owned payloads; they must not be presented as whole-Part sizes.

| Parts | Overrides | Scope | Live requested B | B per Part/container |
|---:|---:|---|---:|---:|
| 300 | 0 | world_total | 575984 | 1919.947 |
| 300 | 0 | named_btree | 7200 | 24.000 |
| 300 | 0 | id_btree | 7200 | 24.000 |
| 300 | 0 | id_sorted_vec | 7200 | 24.000 |
| 300 | 2 | world_total | 797684 | 2658.947 |
| 300 | 2 | named_btree | 228900 | 763.000 |
| 300 | 2 | id_btree | 151200 | 504.000 |
| 300 | 2 | id_sorted_vec | 64800 | 216.000 |
| 300 | 7 | world_total | 810884 | 2702.947 |
| 300 | 7 | named_btree | 242100 | 807.000 |
| 300 | 7 | id_btree | 153900 | 513.000 |
| 300 | 7 | id_sorted_vec | 125100 | 417.000 |
| 5000 | 0 | world_total | 1350864 | 270.173 |
| 5000 | 0 | named_btree | 120000 | 24.000 |
| 5000 | 0 | id_btree | 120000 | 24.000 |
| 5000 | 0 | id_sorted_vec | 120000 | 24.000 |
| 5000 | 2 | world_total | 5045864 | 1009.173 |
| 5000 | 2 | named_btree | 3815000 | 763.000 |
| 5000 | 2 | id_btree | 2520000 | 504.000 |
| 5000 | 2 | id_sorted_vec | 1080000 | 216.000 |
| 5000 | 7 | world_total | 5265864 | 1053.173 |
| 5000 | 7 | named_btree | 4035000 | 807.000 |
| 5000 | 7 | id_btree | 2565000 | 513.000 |
| 5000 | 7 | id_sorted_vec | 2085000 | 417.000 |

On this build, Value is 40 B and an ID/Value pair is 48 B. Seven independent owned Value payloads alone require 280 B before keys/containers/world storage. Prototype shared defaults retain 656 requested B, excluded from each component row.

A mutation journal entry is 16 B. A fully retained 16,384-entry history therefore needs at least 262144 payload B before allocator/capacity overhead. This is a calculated fixed-cost lower bound, not an allocation measurement of the ring implementation.

## Component CPU samples

Six rotated-order samples per case. These are component operations with pre-resolved IDs, not World API or frame timings. Concurrent-build noise prevents treating them as a controlled performance baseline.

| Parts | Overrides | Scope | Read ns/op median [min,max] | Write ns/op median [min,max] |
|---:|---:|---|---|---|
| 300 | 0 | named_btree | 3.09 [2.88,3.42] | 97.90 [90.92,117.40] |
| 300 | 0 | id_btree | 1.24 [1.15,1.34] | 29.88 [27.43,36.62] |
| 300 | 0 | id_sorted_vec | 1.03 [0.90,1.35] | 20.36 [18.67,22.28] |
| 300 | 2 | named_btree | 11.78 [10.00,12.64] | 86.98 [78.80,93.43] |
| 300 | 2 | id_btree | 4.25 [4.17,4.47] | 14.98 [14.85,19.87] |
| 300 | 2 | id_sorted_vec | 2.71 [2.64,2.84] | 10.77 [9.30,11.78] |
| 300 | 7 | named_btree | 27.00 [25.06,109.78] | 110.36 [106.67,132.38] |
| 300 | 7 | id_btree | 6.48 [6.20,6.90] | 15.81 [15.05,16.45] |
| 300 | 7 | id_sorted_vec | 5.73 [5.50,6.35] | 14.59 [13.85,43.62] |
| 5000 | 0 | named_btree | 3.49 [3.06,4.50] | 265.46 [132.64,583.98] |
| 5000 | 0 | id_btree | 1.28 [1.19,19.02] | 41.63 [35.33,57.88] |
| 5000 | 0 | id_sorted_vec | 1.01 [0.92,1.26] | 27.89 [19.52,40.74] |
| 5000 | 2 | named_btree | 27.14 [9.33,42.76] | 283.26 [200.89,332.26] |
| 5000 | 2 | id_btree | 5.35 [4.33,21.56] | 41.51 [17.92,76.34] |
| 5000 | 2 | id_sorted_vec | 5.86 [4.47,13.37] | 14.43 [13.78,52.83] |
| 5000 | 7 | named_btree | 73.25 [64.67,94.11] | 419.25 [381.36,629.33] |
| 5000 | 7 | id_btree | 11.52 [7.75,22.64] | 22.91 [18.98,49.28] |
| 5000 | 7 | id_sorted_vec | 32.37 [12.76,49.38] | 23.42 [19.62,85.93] |
