# インストール元

[← ドキュメント目次](../../README.ja.md) · [English](../en/install-sources.md)

## インストール元の指定方法

`pnl install` は `packages/<name>` のようなローカルパスだけでなく、いくつかの形式で取得元を指定できます。

現在対応している形式です。

```sh
# ローカルの拡張フォルダからインストール。
pnl install /absolute/path/to/extension-root

# file:// URL からインストール。
pnl install file:///absolute/path/to/extension-root

# GitHub リポジトリ内のパッケージフォルダからインストール。
pnl install https://github.com/m3m0r7/pnl-packages/packages/libusb

# GitHub の tree URL からインストール（"main" が clone するブランチになります）。
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# パッケージのサブフォルダを含む scp 形式の SSH URL からインストール。
pnl install git@github.com:m3m0r7/pnl-packages/packages/libusb
```

GitHub の HTTPS URL と scp 形式の SSH URL では、ホスト名のあとの最初の 2 つのパス（`owner/repository`）をリポジトリとして扱い、残りをそのリポジトリ内のパッケージの場所として扱います。`/tree/<branch>/...` を含む GitHub の URL も使えます。この場合は `<branch>` を clone するブランチ、残りをパッケージの場所として扱います。

たとえば次の URL は、

```text
https://github.com/m3m0r7/pnl-packages/packages/libusb
https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
git@github.com:m3m0r7/pnl-packages/packages/libusb
```

次のリポジトリとして clone され、

```text
https://github.com/m3m0r7/pnl-packages.git
git@github.com:m3m0r7/pnl-packages.git
```

次のパッケージフォルダからインストールされます。

```text
packages/libusb
```

clone した内容は一時的にシステムの一時フォルダに置かれます。Linux なら `/tmp/pnl/git/...`、macOS なら `/var/folders/.../T/pnl/git/...` のような場所です。`@pnlx/packages/<vendor>/<package>/<version>` にコピーされるのは、`pnlx.json` を含む、解決済みのパッケージフォルダだけです。

いずれの形式でも、解決されたローカルのパスに `pnlx.json` が無ければインストールは失敗します。

ネイティブライブラリとヘッダーは FTP（`ftp://`）と FTPS（`ftps://`、TLS 上の FTP）からも取得できます。URL に認証情報が無い場合は匿名ログインを使います。
