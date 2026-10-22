# pdf-inspector

PDF の内部構造を調べ、編集するためのツール群。設計は [docs/DESIGN.md](docs/DESIGN.md) を参照。

## 構成（現状）

| crate | 内容 |
|---|---|
| `crates/pdfi-syntax` | ByteSource、lexer / parser、xref（table / stream / hybrid / Prev チェーン）、ObjStm、filters、暗号（RC4 / AES-128 / AES-256）、破損時の復旧、書き出し |
| `crates/pdfi-cli` | `pdfi` コマンド |

## 使い方

```sh
cargo build --release
target/release/pdfi info file.pdf
target/release/pdfi xref file.pdf [--rev N]
target/release/pdfi obj file.pdf 12 [0]
target/release/pdfi stream file.pdf 12 [--raw] [-o out.bin]
target/release/pdfi check file.pdf [--decode] [-v]
target/release/pdfi rewrite in.pdf out.pdf [--reserialize]
```

暗号化ファイルは `--password`、xref を無視して走査で復元するには `--recover`。

## テスト

```sh
cargo test --workspace
tests/fixtures/make.sh                    # フィクスチャ再生成（python3, qpdf が必要）
scripts/corpus-check.sh <dir> [--decode]  # 手元の PDF 群で qpdf とページ数を突き合わせ
```
