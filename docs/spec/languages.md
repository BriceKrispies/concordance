# Languages (B-5)

Each supported language has fixtures, small repositories with a spec and code that disagree in known ways,
and a case table beside each fixture. The requirement states the capability once. The cases state the detail,
and supporting a new construct or a new language adds cases, not requirements.

**LANG-001** A scan of a repository in a supported language SHALL report every finding listed in that language's conformance cases.
- Verify: conformance
- Boundary: B-5 `concordance_engine::start`
- Cases: fixtures/structural-drift/EXPECTED.md, fixtures/undocumented/EXPECTED.md, fixtures/consistent/EXPECTED.md, fixtures/typescript/EXPECTED.md, fixtures/csharp/EXPECTED.md, fixtures/lean/EXPECTED.md
- Traces: G-1, G-4
