//! Integration snapshots for `lithograph inspect artifacts`.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;

#[test]
fn inspect_artifacts_table_fixture_snapshot() -> Result<(), Box<dyn Error>> {
    let output = inspect_artifacts(["inspect", "artifacts", "fixtures/polyglot"])?;

    assert_eq!(output, TABLE_SNAPSHOT);

    Ok(())
}

#[test]
fn inspect_artifacts_json_is_deterministic() -> Result<(), Box<dyn Error>> {
    let first = inspect_artifacts([
        "inspect",
        "artifacts",
        "fixtures/polyglot",
        "--format",
        "json",
    ])?;
    let second = inspect_artifacts([
        "inspect",
        "artifacts",
        "fixtures/polyglot",
        "--format",
        "json",
    ])?;
    let parsed: Value = serde_json::from_str(&first)?;

    assert_eq!(first, second);
    assert_eq!(parsed["artifacts"].as_array().map(Vec::len), Some(23));
    assert_eq!(parsed["artifacts"][0]["path"], ".github/workflows/ci.yml");
    assert_eq!(parsed["artifacts"][0]["category"], "ContinuousIntegration");
    assert_eq!(parsed["artifacts"][10]["path"], "docs/architecture.md");
    assert_eq!(parsed["artifacts"][22]["path"], "web/src/App.tsx");
    assert_eq!(parsed["artifacts"][22]["format"], "tsx");

    Ok(())
}

fn inspect_artifacts<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn Error>> {
    let mut command = Command::cargo_bin("lithograph")?;
    let output = command
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(String::from_utf8(output)?)
}

const TABLE_SNAPSHOT: &str = "\
path                        category               format            support               size  hash                                                              text        model        gen  ven
--------------------------  ---------------------  ----------------  ----------------  --------  ----------------------------------------------------------------  ----------  -----------  ---  ---
.github/workflows/ci.yml    ContinuousIntegration  github-actions    StructuredFormat       458  2b59ffcbd6def7c05818826d6b3a703e20c93e01d6599d59369b74e9e2432698  Text        Allowed        0    0
Dockerfile                  ContainerDefinition    dockerfile        StructuredFormat       466  c18ecb530d308f6a0ccbb7eb72ef6a96f315f27145f48ca12386e7e8c57092bc  Text        Allowed        0    0
LICENSE                     Documentation          license           GenericText            173  6a25e896291c6e8a18b804383fc895115cfc65c20c583931fbf9eda3ee8d9d38  Text        Allowed        0    0
Makefile                    BuildDefinition        makefile          GenericText            272  c1e03eae4cfa721f4bd27bfb03447da3b276503634e4be3f2b848b88344141c3  Text        Allowed        0    0
README.md                   Documentation          markdown          StructuredFormat      3011  38a4f33e81563618c86734a6fa806284ee1273f5cd9932df337585543110b92c  Text        Allowed        0    0
assets/logo.svg             StaticAsset            svg               GenericText            221  40bf5b667bef2bc02ebb2cbcffaa208c006df682bde211fa17fd07e7c9822826  Text        ExcerptOnly    0    0
config/schema.json          Configuration          json              StructuredFormat       363  d60c12520dc000503a4cc35138cd97cf90614d0d7f3f734c58311f58c25f12f0  Text        Allowed        0    0
config/settings.yaml        Configuration          yaml              StructuredFormat       287  3aa612aa2ab42f0c8c4dfa8574f73fb0dfb95edf966e39707aad5c1fb87ef176  Text        Allowed        0    0
data/sample.bin             BinaryAsset            bin               Opaque                  31  e770afdbd274cdfd5c79bc061c2b827007db334662bf3f469a401baff21936b1  Binary      Never          0    0
docker-compose.yml          ContainerDefinition    docker-compose    StructuredFormat       419  efec74f81f225628f7fc1be5bd625090efb22ceb2ef05375296bbfaf12bfffba  Text        Allowed        0    0
docs/architecture.md        Documentation          markdown          StructuredFormat       599  ac0cc5269834bd1d41d2e3e262792d02e1fe6e1d8ecbbee8eb1c1eb4bc2fac79  Text        Allowed        0    0
generated/client.py         GeneratedSource        python            DeepLanguage           129  59bdeaa192e51ada632639b29ffbb731a42a2f384fca383a2396e3ac04083a51  Text        Allowed      100    0
pyproject.toml              PackageManifest        toml              StructuredFormat       162  377e832f866702f3b2a7fec6bd1a68d1f7bf59327f3d540bbcf7f88ef69aca70  Text        Allowed        0    0
requirements.txt            PackageManifest        requirements-txt  GenericText             23  01ac1365b499ea05b197899923bc189567215e613ac7b79f2ebef810c11957df  Text        Allowed        0    0
rust/Cargo.toml             PackageManifest        toml              StructuredFormat       237  47b6bb07b6b067244ef2097353baba82c47a071b6f5c68fe788f68eb8efd76b1  Text        Allowed        0    0
rust/src/bin/worker.rs      SourceCode             rust              DeepLanguage           242  9ec6e1d0b7b81a95e57d14c7844b97c0ffbd705c558a2efc051ff25578f643b9  Text        Allowed        0    0
rust/src/lib.rs             SourceCode             rust              DeepLanguage           797  44e5ed502724f66f09646458be7be4970e502193ef8d801a7fba3ace1a81b0b0  Text        Allowed        0    0
src/python_app/__init__.py  SourceCode             python            DeepLanguage           148  0195b39eb6f54786cd7f39a8933088d1f0ca55f158606bebcbe1a65c4f7711a6  Text        Allowed        0    0
src/python_app/service.py   SourceCode             python            DeepLanguage          1016  47f33f5acc8a1405a414e3d86233b99ab84dd4e4689935a882eda448bb225845  Text        Allowed        0    0
vendor/example/lib.rs       SourceCode             rust              DeepLanguage            61  141b99647806161d5853cb47f24bc1c1e8c9d0a82171e005c90fb44df36b5095  Text        Allowed        0  100
web/index.html              Template               html              StructuredFormat       226  a21fc9744cb5854209131f4da1ddc04e34540c2ae842e5b5c43a3e95630458a5  Text        Allowed        0    0
web/package.json            PackageManifest        npm               StructuredFormat       197  4aced8c963a538dbea1dc59803e09983ffba07a77b41b774b04becf486f071c1  Text        Allowed        0    0
web/src/App.tsx             SourceCode             tsx               StructuredFormat       300  e89fd652d40852d6b652916211423f4ff5bbd23b1f5100a62d0072c9735971c3  Text        Allowed        0    0
";
