# Comment lexer

This specification is normative. Each requirement is one statement, labelled with
its ID, so a test can declare exactly which statement it verifies.

## Block comments

**LEX-001** A block comment SHALL end at the `*/` that balances its opening `/*`.

**LEX-002** The lexer MUST reject a block comment that is never closed.
