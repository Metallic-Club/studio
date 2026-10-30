# A1 checkpoint paired world observations

Exact base: `fbaa17d03d4321b300e6f3ce6d17dc38d5c36223`; candidate: `1176dda264ad6a1ca0a9976f376f636184e43a19`.
Existing world workload is byte-unchanged. Both binaries were built separately with the pinned toolchain and
locked dependency source, retained in owned output lanes without copied compiler/cache artifacts. The entire
planned ABBAABBA sequence completed successfully; all 19 benchmark IDs matched. Settings were fixed before
execution: 20 samples, 0.3-second warmup, 1-second measurement, 10,000 resamples, no plots. Raw estimates and
per-run median confidence intervals are retained under `paired-world/`; no run was removed.

**The design's 5% target is not met by these observed median paired ratios.** Short activity snapshots recorded
74.11% host CPU before and 63.44% after across 12 logical processors, despite the managed team holding builds.
This is not a clean-host or universal performance pass. The large ranges do not establish a product cause or
justify dismissing a regression as noise. Values below are wall-time observations; no instruction-count or
throughput-capacity claim is made. Baseline/candidate medians and the median of pair ratios are separate
statistics, so their displayed quotient need not equal the paired percentage.

| Workload | Baseline median ns (median of 4 runs) | Candidate median ns (median of 4 runs) | Median paired change | Pair range |
|---|---:|---:|---:|---:|
| frame_read/borrowed_seven_props_per_part/300 | 160412.245 | 122660.300 | +3.03% | 0.545 to 1.632 |
| frame_read/borrowed_seven_props_per_part/5000 | 2785234.931 | 3551242.857 | +14.21% | 0.629 to 1.580 |
| frame_read/seven_props_per_part/300 | 169083.816 | 124765.500 | -15.59% | 0.438 to 1.338 |
| frame_read/seven_props_per_part/5000 | 3194213.535 | 3323001.587 | -10.82% | 0.858 to 1.158 |
| property/get_default_value | 55.819 | 62.184 | +0.97% | 0.718 to 1.309 |
| property/get_name | 93.232 | 83.386 | +7.88% | 0.775 to 1.107 |
| property/get_ref_default_value | 86.247 | 80.601 | -4.18% | 0.811 to 1.186 |
| property/get_ref_name | 73.006 | 78.036 | +12.35% | 0.731 to 1.288 |
| property/get_ref_set_value | 24.182 | 32.000 | +0.18% | 0.626 to 1.543 |
| property/get_set_value | 35.095 | 21.368 | -33.74% | 0.500 to 1.148 |
| property/is_a_basepart | 18.254 | 16.067 | +2.08% | 0.579 to 1.263 |
| property/set_position | 154.891 | 170.498 | +18.70% | 0.813 to 1.479 |
| scene/load_5000 | 55133200.000 | 53996000.000 | -8.18% | 0.807 to 1.541 |
| scene/save_5000 | 34172700.000 | 43435512.500 | +14.94% | 0.531 to 1.302 |
| tree/create_1000_parts | 1963461.653 | 1804858.462 | +27.75% | 0.472 to 1.565 |
| tree/deep_clone_5000 | 4782036.346 | 5571846.657 | +31.28% | 0.969 to 1.693 |
| tree/destroy_1000_siblings_one_by_one | 783112.325 | 843264.145 | +0.93% | 0.375 to 2.506 |
| tree/find_first_child_miss_1000 | 77258.676 | 68283.182 | -15.30% | 0.793 to 0.923 |
| tree/full_name | 341.712 | 321.741 | -9.70% | 0.756 to 1.951 |

## Subsequent bounded layout improvement

The initial timing table above belongs to checkpoint `1176dda`, before the following private representation
change. It must not be relabelled as a timing result for the changed candidate.

An explicitly executed manual layout probe on this Windows x86-64 compiler measured the initial history
entry at 40 bytes and optional metadata at 24 bytes. Retaining a private static PropDef pointer for property
events and converting to public Property(def.name) on read reduces those to 32 and 16 bytes respectively.
The public ChangeKind and Change remain 24 and 32 bytes. Warm capacity/retention remain 32,768/15,425 entries;
requested history-buffer storage changes from 1,310,720 to 1,048,576 bytes, a 20% reduction. No retention
algorithm, wire ID dependency, unsafe pointer encoding or public API change is involved.

The existing 5,000-Part requested-heap fixture changed from 1,172.456 to 1,120.027 bytes/Part. This includes a
shared bounded journal and is neither RSS nor a component-size/zero-footprint claim. Warmed 1,000 scalar writes
still request exactly 1,000 allocations / 12,000 bytes, attributable to existing property-map keys, without
an additional per-record name allocation. These observations establish storage/allocation behavior only;
they do not explain the earlier wide timing ratios or establish a timing improvement.

After the representation change, all 56 default core tests passed. One manual layout probe was ignored by
the default run but explicitly executed successfully before and after the change. The design's 5% wall-time
acceptance remains unproven; a narrower mutation/control comparison is planned before claiming any pass.
