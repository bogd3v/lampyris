# Lampyris

*Muchas luces pequeñas e independientes que, juntas, hacen visible la oscuridad.*

[English](README.md)

Lampyris es un **detector de comportamiento inauténtico coordinado** en redes
sociales, **de código abierto y que corre en tu propio computador**. Observa
publicaciones públicas y destaca grupos de cuentas distintas que publican casi
el mismo texto en cuestión de minutos —la huella de redes de bots, campañas de
astroturfing y cadenas de copiar y pegar— y muestra la evidencia para que
cualquiera pueda revisarla.

Todo corre localmente. Sin servidor, sin cuenta, sin llave de API.

> **Un cluster es un patrón que vale la pena revisar, no una prueba de que
> sean bots.** Fans citando una canción, gente compartiendo el mismo titular o
> un meme viral pueden parecerse. Lampyris muestra las señales detrás de cada
> puntaje para que una persona juzgue. Ver
> [docs/METHODOLOGY.md](docs/METHODOLOGY.md).

## Estado

MVP temprano (v0.1). Fuente: **Bluesky**, a través de su firehose público
Jetstream. Plataforma: **Linux**. Interfaz: **línea de comandos** y una app de
escritorio con **Qt y KDE Kirigami**.

## Qué detecta hoy

**Publicación coordinada de textos casi idénticos**: N cuentas distintas
publicando el mismo texto (o casi) dentro de una ventana de tiempo deslizante.

- El texto se normaliza antes de comparar, así que las variaciones cosméticas
  no esconden una campaña: mayúsculas, emojis, enlaces, menciones, tildes,
  letras de ancho completo o "negrita matemática", caracteres cirílicos o
  griegos que imitan letras latinas y caracteres invisibles.
- Funciona en cualquier idioma. En escrituras sin espacios (chino, japonés,
  tailandés…) compara caracteres en vez de palabras.
- Cada cluster trae un puntaje explicable y las señales en bruto: cuentas,
  publicaciones, duración, cuentas por minuto, distancia entre textos y
  proporción de publicaciones con tarjeta de enlace.

Rendimiento en un núcleo de CPU tipo portátil: ~100 mil publicaciones por
segundo solo el detector, ~45 mil de punta a punta incluyendo el JSON. Todo el
firehose de Bluesky es una fracción pequeña de eso.

## Inicio rápido

Requisitos: Rust ≥ 1.85.

```sh
cargo build --release

# Observar el firehose de Bluesky en vivo (Ctrl-C para detener)
./target/release/lampyris watch

# Solo español y portugués, reportar desde 10 cuentas, y grabar el tráfico
./target/release/lampyris watch --lang es --lang pt --min-accounts 10 --record hoy.jsonl

# Volver a correr el detector sobre una grabación (determinista)
./target/release/lampyris replay hoy.jsonl --json > clusters.jsonl

# Clusters guardados localmente (~/.local/share/lampyris/lampyris.db)
./target/release/lampyris list
```

### App de escritorio (Qt 6 + KDE Kirigami)

```sh
cargo run --release -p lampyris-ui
```

Necesita los archivos de desarrollo de Qt 6 y Kirigami de KDE Frameworks 6.
La tabla de paquetes por distribución está en el [README en inglés](README.md#desktop-app-qt-6--kde-kirigami).

## Arquitectura

```
crates/
├── lampyris-core     motor de detección: sin red, sin disco, sin async
├── lampyris-ingest   fuentes: cliente de Jetstream (cursor + reconexión), reproducción JSONL
├── lampyris-storage  SQLite local: solo clusters y referencias, con retención
├── lampyris          pipeline (hilos + canales acotados) y la CLI `lampyris`
└── lampyris-ui       app con Qt Quick + Kirigami vía CXX-Qt
```

## Hoja de ruta

- **Señales de comportamiento** que funcionen aunque el texto lo genere una IA
  y no sea idéntico: creación de cuentas en ráfaga, horarios, grafos de
  amplificación.
- **Más fuentes**: Mastodon y el fediverso, canales públicos de Telegram.
- **Reportes firmados y verificables** contra los datos públicos.
- **Federación** entre nodos autoalojados que intercambian hallazgos firmados.
- **Versión WebAssembly** del núcleo para usar desde el navegador.
- Empaquetado en Flatpak.

## Contribuir

Cualquiera puede contribuir, también con seudónimo. Ver
[CONTRIBUTING.md](CONTRIBUTING.md).

## Licencia

GPL-3.0 o posterior. Ver [LICENSE](LICENSE).
