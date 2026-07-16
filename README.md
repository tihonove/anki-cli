# anki-cli

Git-подобный CLI для Anki: локальная копия коллекции, правки из терминала (или от агента), синхронизация с AnkiWeb с явным разрешением конфликтов.

Собран поверх официального `rslib` (Rust-ядра Anki, [ankitects/anki](https://github.com/ankitects/anki)) — то есть база данных, схема и протокол синхронизации ровно те же, что у настоящего Anki. Один самостоятельный бинарник.

## Ментальная модель (как git)

| git | anki-cli | что делает |
|---|---|---|
| `clone` / `reset --hard origin` | `pull` | полная загрузка коллекции с сервера (затирает локальную) |
| `push --force` | `push` | полная заливка локальной коллекции на сервер (затирает серверную) |
| `pull` + `push` (merge) | `sync` | обычная двусторонняя синхронизация; **exit code 2** при конфликте |
| `status` | `status` | локальные изменения + отличается ли сервер |

Конфликт (после смены схемы, `push` с другого устройства и т.п.) обычным `sync` не решается — команда печатает варианты и выходит с кодом 2. Решение: `pull` (взять серверную версию) или `push` (взять локальную).

`pull` откажется затирать несинканные локальные изменения — добавьте `--force`, чтобы согласиться на потерю.

## Быстрый старт

```bash
anki-cli login -u you@example.com -p 'password'   # или env: ANKI_USERNAME / ANKI_PASSWORD
anki-cli pull                                     # забрать коллекцию с AnkiWeb

anki-cli add -d "Deutsch::A1" "der Hund" "собака" -t "noun a1"
anki-cli add -m "Basic (and reversed card)" --field Front="die Katze" --field Back="кошка"
anki-cli add -m Cloze "Der {{c1::Hund}} bellt."

anki-cli sync                                     # залить изменения (двусторонний merge)
```

## Команды

```
login -u <email> -p <pass> [--endpoint URL]   логин, ключ сессии сохраняется в config.json
logout                                        забыть ключ
status [--offline]                            заметки/карточки, локальные изменения, статус сервера
sync                                          двусторонняя синхронизация (exit 2 = конфликт)
pull [--force]                                полная загрузка с сервера
push                                          полная заливка на сервер

add [-d DECK] [-m MODEL] [значения полей...] [--field Имя=Значение]... [-t "теги"]
search <запрос> [--limit N]                   поисковый синтаксис Anki: deck:X tag:Y слово
show <note_id>                                заметка целиком
edit <note_id> [--field Имя=Значение]... [--add-tags "..."] [--remove-tags "..."]
rm <note_id>...                               удалить заметки (с их карточками)
decks                                         список колод с числом карточек
models [имя]                                  список типов заметок / поля конкретного типа
```

Глобальные флаги:

- `--json` — машиночитаемый вывод (для агентов); ошибки в JSON уходят в stderr.
- `--dir PATH` — каталог данных (коллекция + config). По умолчанию `$ANKI_CLI_HOME`, иначе `~/.local/share/anki-cli`.

## Пример: рабочий цикл агента

```bash
anki-cli pull
anki-cli --json search "deck:Spanish tag:verb"       # что уже есть
anki-cli --json add -d Spanish "el perro" "собака" -t noun
anki-cli sync || {
  # exit 2: коллекции разошлись — решаем в пользу локальной версии
  anki-cli push
}
```

## Сборка

Требуется исходное дерево Anki рядом с проектом (rslib подключается по пути `../anki-src/rslib`):

```bash
git clone --depth 1 https://github.com/ankitects/anki.git ../anki-src
# rslib читает переводы из git-сабмодулей; для english-only сборки хватает пустых папок:
mkdir -p ../anki-src/ftl/core-repo/core ../anki-src/ftl/qt-repo/desktop
# нужен protoc (см. .cargo/config.toml — там задаётся PROTOC):
#   либо apt install protobuf-compiler, либо бинарь с github.com/protocolbuffers/protobuf/releases

cargo build --release          # бинарник в target/release/anki-cli
cargo test                     # локальные интеграционные тесты (без сети)
```

## Что внутри / ограничения

- Локальная коллекция — обычный `collection.anki2` (SQLite, схема Anki). Его можно открыть настольным Anki, если очень хочется.
- Ключ сессии (hkey) хранится в `config.json` в каталоге данных в открытом виде — как и у настольного Anki. Пароль не сохраняется.
- AnkiWeb перенаправляет на шард (например `sync11.ankiweb.net`) — CLI сам подхватывает и запоминает эндпоинт.
- Синхронизация **медиа-файлов пока не реализована** (картинки/аудио в заметках синкаются как текстовые ссылки, сами файлы — нет).
- Изучение карточек (scheduler/review) в CLI не выведено — предполагается, что учитесь вы в обычном Anki, а CLI служит для наполнения и синка.
- Лицензия: rslib — AGPL-3.0, соответственно и этот инструмент — AGPL-3.0.
