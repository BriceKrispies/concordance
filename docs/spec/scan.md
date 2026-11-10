# Every scan (B-5)

**SCAN-001** A scan SHALL leave every file under the analyzed repository byte-identical.
- Verify: property
- Boundary: B-5 `concordance_engine::start`
- Traces: K-1

**SCAN-002** Two scans of the same repository content SHALL report the same findings.
- Verify: property
- Boundary: B-5 `concordance_engine::start`
- Traces: K-2
