# Wire format

This page describes the bytes that go over the connection. It is background
reading; the normative rules live in the specification.

## Header lines

Each header is sent on its own line as `name: value`, terminated by a newline.

## Versioning

The wire format carries no version field. Peers agree on the format out of band.
