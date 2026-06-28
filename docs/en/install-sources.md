# Install Sources

[← Documentation index](../../README.md) · [日本語](../ja/install-sources.md)

## Install Sources

`pnl install` accepts more than just `packages/<name>` local paths — you can specify the source in several forms.

Currently supported:

```sh
# Install from a local extension folder.
pnl install /absolute/path/to/extension-root

# Install from a file:// URL.
pnl install file:///absolute/path/to/extension-root

# Install from a package folder inside a GitHub repository.
pnl install https://github.com/m3m0r7/pnl-packages/packages/libusb

# Install from a GitHub tree URL ("main" becomes the branch to clone).
pnl install https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb

# Install from an scp-style SSH URL with a package subfolder.
pnl install git@github.com:m3m0r7/pnl-packages/packages/libusb
```

For GitHub HTTPS URLs and scp-style SSH URLs, the first two path segments after the host (`owner/repository`) are treated as the repository, and the rest is treated as the package's location inside it. GitHub URLs using `/tree/<branch>/...` are also accepted; there, `<branch>` is the branch to clone and the rest is the package location.

For example, these URLs:

```text
https://github.com/m3m0r7/pnl-packages/packages/libusb
https://github.com/m3m0r7/pnl-packages/tree/main/packages/libusb
git@github.com:m3m0r7/pnl-packages/packages/libusb
```

are cloned as:

```text
https://github.com/m3m0r7/pnl-packages.git
git@github.com:m3m0r7/pnl-packages.git
```

and installed from:

```text
packages/libusb
```

The clone is placed temporarily in the system temp directory — somewhere like `/tmp/pnl/git/...` on Linux, or `/var/folders/.../T/pnl/git/...` on macOS. Only the resolved package folder that contains `pnlx.json` is copied into `@pnlx/packages/<vendor>/<package>/<version>`.

In every case, install fails if the resolved local path does not contain `pnlx.json`.

Native libraries and headers can also be fetched over FTP (`ftp://`) and FTPS (`ftps://`, FTP over TLS); anonymous login is used unless the URL carries credentials.
