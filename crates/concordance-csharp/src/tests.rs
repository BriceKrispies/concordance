use crate::CSharpPlugin;
use concordance_core::code::*;
use concordance_core::lang::LanguagePlugin;
use concordance_core::RepoPath;

fn ws() -> WorkspaceInfo {
    WorkspaceInfo {
        crates: vec![CrateInfo {
            name: "Volume".into(),
            language: "csharp".into(),
            manifest: RepoPath::from_relative_str("Volume.csproj").unwrap(),
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
    CSharpPlugin.index(&RepoPath::from_relative_str(path).unwrap(), text, &ws())
}

const SRC: &str = r#"using System;
using static System.Math;

namespace Acme.Audio;

/// <summary>A mixer channel.</summary>
/// <remarks>Verifies: VOL-009</remarks>
public class Mixer
{
    public const int MaxVolume = 100;
    private int _level;
    protected internal int Shared;

    public event EventHandler Changed;

    /// <summary>Sets the volume.</summary>
    /// <param name="v">The level.</param>
    public void SetVolume(int v)
    {
        if (v > 100) { throw new ArgumentOutOfRangeException(nameof(v)); }
        _level = Math.Clamp(v, 0, 100);
        var home = Environment.GetEnvironmentVariable("HOME");
    }

    public int Level { get; private set; }

    public int Doubled => _level * 2;

    void Hidden() { }

    unsafe void Raw() { }

    public Mixer(int level) { _level = level; }
}

internal interface IThing { void Run(); }

public enum Mode { Off, On }

public record Point(int X, int Y);

public struct Pair { public int A; }

public delegate void Handler(int code);
"#;

fn find<'a>(idx: &'a SourceIndex, qn: &str) -> &'a Symbol {
    idx.symbols
        .iter()
        .find(|s| s.qualified_name() == qn)
        .unwrap_or_else(|| {
            panic!(
                "no {qn}; have {:?}",
                idx.symbols
                    .iter()
                    .map(|s| s.qualified_name())
                    .collect::<Vec<_>>()
            )
        })
}

#[test]
fn indexes_declarations_with_csharp_visibility() {
    let idx = index("src/Mixer.cs", SRC);
    assert_eq!(idx.file.language, "csharp");
    assert_eq!(idx.file.crate_name.as_deref(), Some("Volume"));
    assert!(idx.file.parse_error.is_none());

    assert_eq!(
        find(&idx, "Volume::Acme::Audio").kind,
        SymbolKind::Namespace
    );
    let mixer = find(&idx, "Volume::Acme::Audio::Mixer");
    assert_eq!(mixer.kind, SymbolKind::Class);
    assert!(mixer.is_public_api);
    assert!(mixer.doc.as_deref().unwrap().contains("A mixer channel."));
    assert_eq!(mixer.verifies, vec!["VOL-009".to_string()]);

    let set = find(&idx, "Volume::Acme::Audio::Mixer::SetVolume");
    assert_eq!(set.kind, SymbolKind::Method);
    assert_eq!(set.visibility, Visibility::Public);
    assert!(set.is_public_api);
    assert_eq!(set.signature.as_deref(), Some("void SetVolume(int v)"));
    assert_eq!(
        set.doc
            .as_deref()
            .map(|d| d.starts_with("Sets the volume.")),
        Some(true)
    );

    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mixer::_level").visibility,
        Visibility::Private
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mixer::Shared").visibility,
        Visibility::Restricted
    );
    let hidden = find(&idx, "Volume::Acme::Audio::Mixer::Hidden");
    assert_eq!(hidden.visibility, Visibility::Private);
    assert!(!hidden.is_public_api);
    let level = find(&idx, "Volume::Acme::Audio::Mixer::Level");
    assert_eq!(level.kind, SymbolKind::Property);
    assert_eq!(
        level.signature.as_deref(),
        Some("int Level { get; private set; }")
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mixer::Doubled")
            .signature
            .as_deref(),
        Some("int Doubled { get; }")
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mixer::Mixer").kind,
        SymbolKind::Constructor
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mixer::Changed")
            .signature
            .as_deref(),
        Some("event EventHandler Changed")
    );

    let max = find(&idx, "Volume::Acme::Audio::Mixer::MaxVolume");
    assert_eq!(max.kind, SymbolKind::Const);
    assert!(max
        .facts
        .iter()
        .any(|f| f.kind == FactKind::ConstValue && f.detail == "MaxVolume = 100"));

    // Defaults: internal types, public interface and enum members.
    let thing = find(&idx, "Volume::Acme::Audio::IThing");
    assert_eq!(thing.kind, SymbolKind::Interface);
    assert_eq!(thing.visibility, Visibility::Crate);
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::IThing::Run").visibility,
        Visibility::Public
    );
    assert!(!find(&idx, "Volume::Acme::Audio::IThing::Run").is_public_api);
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Mode::On").kind,
        SymbolKind::Variant
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Point").kind,
        SymbolKind::Class
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Pair").kind,
        SymbolKind::Struct
    );
    assert_eq!(
        find(&idx, "Volume::Acme::Audio::Handler")
            .signature
            .as_deref(),
        Some("delegate void Handler(int code)")
    );
    assert!(idx.imports.iter().any(|i| i.path == ["System"] && i.glob));
}

#[test]
fn collects_facts() {
    let idx = index("src/Mixer.cs", SRC);
    let set = find(&idx, "Volume::Acme::Audio::Mixer::SetVolume");
    let has = |k: FactKind| set.facts.iter().any(|f| f.kind == k);
    assert!(has(FactKind::PanicSite));
    assert!(has(FactKind::LiteralComparison));
    assert!(has(FactKind::Clamp));
    assert!(has(FactKind::EnvRead));
    assert!(find(&idx, "Volume::Acme::Audio::Mixer::Raw")
        .facts
        .iter()
        .any(|f| f.kind == FactKind::UnsafeCode));
}

#[test]
fn detects_tests_and_verifies_comments() {
    let src = r#"namespace T;

public class MixerTests
{
    // Verifies: VOL-001
    [Fact]
    public void A() { }

    /// <summary>Verifies: VOL-002, VOL-003</summary>
    [Theory]
    [InlineData(1)]
    public void B(int x) { }

    [TestMethod] public void C() { }

    public void NotATest() { }
}
"#;
    let idx = index("src/MixerTests.cs", src);
    let a = find(&idx, "Volume::T::MixerTests::A");
    assert!(a.is_test);
    assert_eq!(a.verifies, vec!["VOL-001".to_string()]);
    let b = find(&idx, "Volume::T::MixerTests::B");
    assert!(b.is_test);
    assert_eq!(
        b.verifies,
        vec!["VOL-002".to_string(), "VOL-003".to_string()]
    );
    assert!(find(&idx, "Volume::T::MixerTests::C").is_test);
    assert!(!find(&idx, "Volume::T::MixerTests::NotATest").is_test);
}

#[test]
fn documented_signatures_match_indexed_ones() {
    let doc = "public static int ParseLevel(string text, int radix = 10);";
    let (name, sig) = CSharpPlugin.normalize_signature(doc).unwrap();
    assert_eq!(name, "ParseLevel");
    let idx = index(
        "src/M.cs",
        "class M { public static int ParseLevel(string text,\n int radix=10) { return 1; } }",
    );
    let actual = find(&idx, "Volume::M::ParseLevel")
        .signature
        .clone()
        .unwrap();
    assert_eq!(
        CSharpPlugin.strip_visibility(&sig),
        CSharpPlugin.strip_visibility(&actual)
    );

    let sigs = CSharpPlugin.documented_signatures("public int Total(int a, int b);\n");
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].name, "Total");
    assert_eq!(sigs[0].start, 0);
}
