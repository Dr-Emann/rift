# Rift vs Mountebank Benchmark Report

**Date:** 2025-11-25 01:09:23
**Duration per test:** 15s
**Concurrent connections:** 50

## Summary Results

| Test | MB RPS | Rift RPS | Speedup |
|------|--------|----------|---------|
| Simple: Health Check | 1914.1590 | 39127.5441 | **20.4x faster** |
| Simple: Ping/Pong | 1763.1616 | 36391.9437 | **20.6x faster** |
| Admin: List Imposters | 6361.9965 | 29021.9876 | **4.5x faster** |
| Admin: Get Imposter | 396.2859 | 884.5531 | **2.2x faster** |
| API: First Stub Match | 1865.9826 | 33422.6712 | **17.9x faster** |
| API: Middle Stub Match | 589.6590 | 25512.0135 | **43.2x faster** |
| API: Last Stub Match | 290.6818 | 22042.2397 | **75.8x faster** |
| API: No Match (404) | 297.8424 | 22664.5233 | **76.0x faster** |
| Regex: First Pattern | 1560.5463 | 7040.5822 | **4.5x faster** |
| Regex: Middle Pattern | 118.9688 | 257.4926 | **2.1x faster** |
| Regex: Last Pattern | 65.2654 | 130.6267 | **2.0x faster** |
| Complex: AND/OR Predicates | 904.8538 | 29322.6051 | **32.4x faster** |
| JSON: Body Equals (First) | 1794.0121 | 29252.5034 | **16.3x faster** |
| JSON: Body Equals (Middle) | 1022.6158 | 24052.8495 | **23.5x faster** |
| JSON: Body Contains | 1272.2644 | 24352.8136 | **19.1x faster** |
| JSONPath: First Match | 107.5234 | 26583.7153 | **247.2x faster** |
| JSONPath: Middle Match | 129.8760 | 28751.5887 | **221.3x faster** |
| JSONPath: Last Match | 124.1842 | 27070.2658 | **217.9x faster** |
| XPath: First Match | 169.4493 | 28745.0847 | **169.6x faster** |
| XPath: Middle Match | 168.9096 | 27234.6470 | **161.2x faster** |
| XPath: Last Match | 173.8784 | 27248.3339 | **156.7x faster** |
| Template: Simple | 1856.1939 | 26858.2333 | **14.4x faster** |
| Template: With Query | 1349.4976 | 28158.2180 | **20.8x faster** |
| Decorate: First | 1759.0810 | 6141.2078 | **3.4x faster** |
| Decorate: Middle | 1640.8110 | 5980.1804 | **3.6x faster** |
| Header: First Route | 1763.2439 | 27937.0178 | **15.8x faster** |
| Header: Middle Route | 1199.3209 | 29345.4356 | **24.4x faster** |
| Header: Last Route | 843.0558 | 28971.3764 | **34.3x faster** |
| Query: First Match | 1787.0860 | 28549.5096 | **15.9x faster** |
| Query: Middle Match | 1143.3337 | 24425.7665 | **21.3x faster** |
| Query: Last Match | 801.7622 | 21185.2385 | **26.4x faster** |
| Stress: 200 Concurrent | 1830.6695 | 29716.6817 | **16.2x faster** |

## Latency Comparison (P99)

| Test | MB P99 | Rift P99 |
|------|--------|----------|
| Simple: Health Check | in | in |
| Simple: Ping/Pong | in | in |
| Admin: List Imposters | in | in |
| Admin: Get Imposter | in | in |
| API: First Stub Match | in | in |
| API: Middle Stub Match | in | in |
| API: Last Stub Match | in | in |
| API: No Match (404) | in | in |
| Regex: First Pattern | in | in |
| Regex: Middle Pattern | in | in |
| Regex: Last Pattern | in | in |
| Complex: AND/OR Predicates | in | in |
| JSON: Body Equals (First) | in | in |
| JSON: Body Equals (Middle) | in | in |
| JSON: Body Contains | in | in |
| JSONPath: First Match | in | in |
| JSONPath: Middle Match | in | in |
| JSONPath: Last Match | in | in |
| XPath: First Match | in | in |
| XPath: Middle Match | in | in |
| XPath: Last Match | in | in |
| Template: Simple | in | in |
| Template: With Query | in | in |
| Decorate: First | in | in |
| Decorate: Middle | in | in |
| Header: First Route | in | in |
| Header: Middle Route | in | in |
| Header: Last Route | in | in |
| Query: First Match | in | in |
| Query: Middle Match | in | in |
| Query: Last Match | in | in |
| Stress: 200 Concurrent | in | in |

## Configuration

- **Imposters:** 12 (API Server, Regex, Complex Predicates, Behaviors, JSON Body, JSONPath, XPath, Templates, Header Routing, Query Params, Decorate, Simple Baseline)
- **Total Stubs:** ~1140+ stubs across all imposters
- **Resource Limits:** 2 CPUs, 1GB RAM per service
