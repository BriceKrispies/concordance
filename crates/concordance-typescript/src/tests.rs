use crate::TypeScriptPlugin;
use concordance_core::code::*;
use concordance_core::lang::LanguagePlugin;
use concordance_core::RepoPath;

fn ws() -> WorkspaceInfo {
    WorkspaceInfo {
        crates: vec![CrateInfo {
            name: "volume".into(),
            language: "typescript".into(),
            manifest: RepoPath::from_relative_str("package.json").unwrap(),
            root_dir: None,
            targets: vec![],
            dependencies: vec![],
            features: vec![],
        }],
        from_cargo_metadata: false,
        warnings: vec![],
    }
}

fn index(path: &str, text: &str) -> SourceIndex {
    TypeScriptPlugin.index(&RepoPath::from_relative_str(path).unwrap(), text, &ws())
}

const SRC: &str = r#"import { clamp as c } from "./util";
import lodash from "lodash";

export const MAX_VOLUME = 100;

/**
 * A mixer channel.
 */
export class Mixer {
  private level = 0;

  constructor(initial: number) {
    this.level = initial;
  }

  /** Sets the volume. */
  setVolume(v: number): void {
    if (v > 150) {
      throw new RangeError("too loud");
    }
    this.level = Math.min(v, 150);
  }

  get volume(): number {
    return this.level;
  }
}

export interface Channel {
  name: string;
  mute(on: boolean): void;
}

export function parseLevel(text: string, radix = 10,): number {
  const port = process.env.MIXER_PORT;
  return parseInt(text, radix) as any;
}

export const scale = async (v: number): Promise<number> => v * 2;

function helper() {}

export enum Mode { Mono, Stereo }

export type Level = number | undefined;
"#;

#[test]
fn indexes_declarations_with_paths_signatures_and_visibility() {
    let idx = index("src/mixer.ts", SRC);
    assert_eq!(idx.file.language, "typescript");
    assert_eq!(idx.file.module_path, vec!["mixer"]);
    let sym = |name: &str| {
        idx.symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    };

    let mixer = sym("Mixer");
    assert_eq!(mixer.kind, SymbolKind::Class);
    assert_eq!(mixer.path, vec!["volume", "mixer", "Mixer"]);
    assert!(mixer.is_public_api);
    assert_eq!(mixer.doc.as_deref(), Some("A mixer channel."));

    let set = sym("setVolume");
    assert_eq!(set.kind, SymbolKind::Method);
    assert_eq!(set.path, vec!["volume", "mixer", "Mixer", "setVolume"]);
    assert_eq!(set.signature.as_deref(), Some("setVolume(v: number): void"));
    assert_eq!(set.doc.as_deref(), Some("Sets the volume."));
    assert!(set
        .facts
        .iter()
        .any(|f| f.kind == FactKind::PanicSite && f.detail.starts_with("throw new RangeError")));
    assert!(set
        .facts
        .iter()
        .any(|f| f.kind == FactKind::LiteralComparison && f.detail == "v>150"));
    assert!(set.facts.iter().any(|f| f.kind == FactKind::Clamp));
    assert_eq!(
        set.range.start_line, 17,
        "range starts at the method, not its doc"
    );

    assert_eq!(sym("level").visibility, Visibility::Private);
    assert_eq!(sym("constructor").kind, SymbolKind::Constructor);
    assert_eq!(
        sym("volume").signature.as_deref(),
        Some("get volume(): number")
    );

    let parse = sym("parseLevel");
    assert_eq!(
        parse.signature.as_deref(),
        Some("function parseLevel(text: string, radix = 10): number"),
        "trailing comma dropped, export dropped"
    );
    assert!(parse
        .facts
        .iter()
        .any(|f| f.kind == FactKind::EnvRead && f.detail == "process.env.MIXER_PORT"));
    assert!(parse
        .facts
        .iter()
        .any(|f| f.kind == FactKind::UnsafeCode && f.detail == "any"));
    assert!(parse.calls.iter().any(|c| c.target
        == CallTarget::Path {
            segments: vec!["parseInt".into()]
        }));

    assert_eq!(
        sym("scale").signature.as_deref(),
        Some("async function scale(v: number): Promise<number>")
    );
    assert!(!sym("helper").is_public_api);
    assert_eq!(sym("mute").kind, SymbolKind::Method);
    assert_eq!(sym("Channel").kind, SymbolKind::Interface);
    assert_eq!(sym("Stereo").kind, SymbolKind::Variant);
    assert_eq!(sym("Level").kind, SymbolKind::TypeAlias);
    let max = sym("MAX_VOLUME");
    assert!(max
        .facts
        .iter()
        .any(|f| f.kind == FactKind::ConstValue && f.detail == "MAX_VOLUME = 100"));

    assert!(idx
        .imports
        .iter()
        .any(|i| i.path == vec!["util", "clamp"] && i.alias.as_deref() == Some("c")));
    assert!(idx.imports.iter().any(|i| i.path == vec!["lodash"]));
}

#[test]
fn tests_are_symbols_with_their_requirements() {
    let src = r#"import { describe, it, expect } from "vitest";

describe("block comments", () => {
  // Verifies: LEX-001
  it("nest", () => {
    expect(1).toBe(1);
  });

  /** Verifies: LEX-002, LEX-003 */
  test("unterminated is refused", () => {});
});

it("top level", () => {});
"#;
    let idx = index("src/lexer.test.ts", src);
    let tests: Vec<&Symbol> = idx.symbols.iter().filter(|s| s.is_test).collect();
    assert_eq!(tests.len(), 3);
    let nest = tests.iter().find(|s| s.name == "nest").unwrap();
    assert_eq!(
        nest.path,
        vec!["volume", "lexer.test", "block comments", "nest"]
    );
    assert_eq!(nest.verifies, vec!["LEX-001"]);
    let un = tests
        .iter()
        .find(|s| s.name == "unterminated is refused")
        .unwrap();
    assert_eq!(un.verifies, vec!["LEX-002", "LEX-003"]);
    assert!(tests
        .iter()
        .find(|s| s.name == "top level")
        .unwrap()
        .verifies
        .is_empty());
}

#[test]
fn documented_signatures_normalize_like_code() {
    let docs = TypeScriptPlugin.documented_signatures(
        "export function parseLevel(text: string,\n    radix = 10): number;\n\nclass Mixer {\n  setVolume(v: number): void;\n}\n",
    );
    let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["parseLevel", "Mixer", "setVolume"]);
    assert_eq!(
        docs[0].text,
        "function parseLevel(text: string, radix = 10): number"
    );
    assert_eq!((docs[0].start, docs[0].end), (0, 1));
    assert_eq!(docs[0].name_col, 16);
    assert_eq!(docs[2].text, "setVolume(v: number): void");

    let (name, sig) = TypeScriptPlugin
        .normalize_signature("setVolume(v: number): void")
        .unwrap();
    assert_eq!(
        (name.as_str(), sig.as_str()),
        ("setVolume", "setVolume(v: number): void")
    );
    let (_, f) = TypeScriptPlugin
        .normalize_signature("function parseLevel(text: string, radix = 10): number")
        .unwrap();
    assert_eq!(f, "function parseLevel(text: string, radix = 10): number");
}

#[test]
fn a_file_that_does_not_parse_has_no_symbols_and_an_error() {
    let idx = index("src/broken.ts", "export function (((");
    assert!(idx.symbols.is_empty());
    assert!(idx.file.parse_error.is_some());
}
