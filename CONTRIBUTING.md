# Разработка YAP

YAP использует Tauri 2, Rust, Preact и TypeScript. Аудио воспроизводится в Rust, а локальный каталог хранится в SQLite. Архитектура, сценарии и ссылки на фреймы Pixso собраны в [плане рефакторинга](REFACTOR_PLAN.md).

## Запуск и проверки

Установи Bun, Rust и [системные зависимости Tauri](https://v2.tauri.app/start/prerequisites/) для своей платформы.

```sh
bun install --frozen-lockfile
bun run tauri dev
```

На Windows можно запустить `bun x tauri dev` из PowerShell. Команда `bun run tauri dev` использует обёртку для настройки звука в WSL.

```sh
bun run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

Workflow **Desktop checks** проверяет фронтенд и Rust на Windows, Linux и macOS.

## Сборка установщиков

### Linux x86_64

На Ubuntu 22.04 установи зависимости и собери пакеты.

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev patchelf libfuse2
bun run build:linux
```

`.deb` и `.AppImage` появятся в `src-tauri/target/release/bundle/{deb,appimage}/`. В CI пакет для Arch Linux собирается через `makepkg` из файлов того же `.deb`. Рецепт и скрипт сборки находятся в [packaging/arch](packaging/arch).

### macOS

Для универсальной сборки нужны инструменты командной строки Xcode и обе цели Rust.

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bun run build:macos
```

DMG появится в `src-tauri/target/universal-apple-darwin/release/bundle/dmg/`. Внутри находится приложение с ad-hoc подписью.

### Windows x64

На Windows используй `bun run build:windows:native`. Для кросс-сборки из WSL подготовь инструменты и запусти отдельную команду.

```sh
./scripts/setup-wsl-windows-build.sh
bun run build:windows
```

Установщик NSIS появится в `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/`.

## CI, релизы и подпись обновлений

Чтобы получить тестовые артефакты, запусти **Desktop builds** вручную с пустым полем **Release tag**. Режим Linux-only собирает `.deb`, AppImage и пакет Arch.

Для релиза workflow берёт версию из тега `vX.Y.Z`. Его можно запустить пушем тега или вручную, выбрав этот тег в качестве ref и указав его в **Release tag**.

```sh
git tag v0.1.0
git push origin v0.1.0
```

CI подписывает артефакты обновления Tauri-ключом из секрета `TAURI_SIGNING_PRIVATE_KEY`. Если у ключа есть пароль, укажи его в `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Для локальной подписанной сборки доступны команды `bun run build:macos:release`, `bun run build:linux:release` и `bun run build:windows:release` с теми же переменными окружения.

Для своего форка сгенерируй пару ключей командой `bun x tauri signer generate -w .tauri/signing.key --ci`. Сохрани закрытый ключ в секрете репозитория. Укажи открытый ключ в `plugins.updater.pubkey` и URL своего релиза в `plugins.updater.endpoints` в [tauri.conf.json](src-tauri/tauri.conf.json). [Документация Tauri Updater](https://v2.tauri.app/plugin/updater/)

После сборки всех пакетов CI создаёт черновик GitHub Release с установщиками, подписями `.sig` и общим манифестом `latest.json`. Публикация черновика делает новую версию доступной установленным копиям YAP.

## Звук в WSL

`bun run tauri dev` использует сокет WSLg `/mnt/wslg/PulseServer`, если он доступен. Обёртка также проверяет локальный сокет PulseAudio и порт `127.0.0.1:4713`. Если подключения нет, используется ALSA null. Для вывода звука через отдельный сервер можно запустить PulseAudio на Windows и открыть порт `4713` для WSL.

Обёртка отключает плагины PipeWire в GStreamer и выставляет `PULSE_LATENCY_MSEC=150`, если значение ещё не задано. Активный аудиовыход и параметры маршрутизации видны в диагностике приложения. Воспроизведение в WSL идёт через Rust, а нормализация громкости использует теги ReplayGain.
