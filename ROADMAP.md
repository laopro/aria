# Aria: lista de pendientes

## 1. Login y sesión
- [ ] Login con OAuth de dispositivo (flujo tipo `ytmusicapi`): Aria muestra un código, el usuario lo confirma en google.com/device. Sin cookies ni webview. Decisión tomada: esta es la vía principal.
- [x] Botón "Sign in" en la UI (sidebar o F8): abre el navegador y espera la sesión. Falta "Sign out" y probarlo con cuenta real.
- [ ] Detectar sesión expirada y mostrarla en la UI, no solo en el log.
- [ ] Mostrar nombre o avatar de la cuenta cuando hay sesión.
- [x] Sesión guardada con permisos `0600` (login desde la UI).

## 2. Sincronización
- [ ] Confirmar que Me gusta (`LM`) e Historial (`FEmusic_history`) cargan con sesión real.
- [ ] Suscripciones, álbumes y artistas guardados.
- [ ] Paginación completa y caché local para no pedir todo en cada arranque.

## 3. Búsqueda
- [x] Las playlists de resultados cargan sus canciones solo al abrirlas.
- [x] Una sola petición de canciones por búsqueda, en paralelo con la general.

## 4. Mouse
- [ ] Doble clic para reproducir.
- [ ] Clic derecho con menú: reproducir después, agregar a la cola, me gusta, ir al artista.
- [ ] Barra de scroll visible en las listas.
- [ ] Arrastrar para reordenar la cola.

## 5. Teclado
- [ ] Tecla `?` con ayuda de atajos.
- [ ] Atajos configurables en un archivo.
- [ ] Confirmar antes de borrar (la tecla `r` borra sin preguntar).
- [ ] Foco visible en la lista o botón seleccionado.

## 6. Reproducción
- [x] Panel de cola a la derecha (F6 o botón Queue; clic para reproducir).
- [ ] Shuffle y repeat.
- [ ] Recordar volumen y última canción al arrancar.
- [ ] Controles de media del sistema (MPRIS en Linux) y notificación al cambiar de canción.

## 7. Robustez
- [ ] Mensaje claro si falta `yt-dlp`, con cómo instalarlo, y opción de actualizarlo.
- [ ] Errores como notificaciones (toast) en vez de la línea de estado.
- [ ] Log en una ruta conocida; `--files` la muestra.

## 8. Proyecto
- [x] README con dependencias de build en Linux (Debian y Fedora).
- [ ] Tests para el parser de la API y los atajos de teclado.

## 9. Rendimiento (hecho)
- [x] Descarga de audio por rangos: YouTube limitaba la descarga normal a ~40 KB/s (~90 s por canción); ahora ~0.7 s.
- [x] Sin segunda descarga a disco de la canción que ya se está transmitiendo.
- [x] URL de stream en caché 1 h (repetir una canción no vuelve a llamar a yt-dlp).
- [x] Redibujado solo con cambios; tick de 1 s en pausa o sin cola.
- [ ] Resolver la URL sin yt-dlp (hoy ~2.5 s por canción nueva; la API directa pide PO token y da 403).
- [x] RAM: 43 MB -> 14 MB en reposo y 64 MB -> 20 MB reproduciendo (asignador del sistema en Linux, un solo heap, `malloc_trim` cada 20 s).
- [x] La URL de la fila seleccionada en la búsqueda se resuelve por adelantado (~1 s hasta sonar si descansas en la fila).
- [x] Reintento con URL nueva cuando YouTube responde 403.
- [ ] Canciones "no disponibles": yt-dlp no tiene runtime de JavaScript configurado y solo puede usar un cliente; muchas pistas oficiales fallan. Instalar deno o `yt-dlp-ejs`.

## 10. Login (estado)
- [x] El botón F8 lee la sesión de Firefox vía yt-dlp (probado: 37 playlists, Historial y Liked Music).

## 11. v0.3.0 (hecho)
- [x] Listas largas completas: Liked Music pasa de 100 a todas sus canciones (paginación nueva de YouTube Music, máx. 3 playlists a la vez y reintentos).
- [x] Scrollbars en todas las listas y en la cola.
- [x] `install.sh` baja también `yt-dlp` si falta.
- [ ] Windows y macOS: por ahora, compilar desde el código.

## 12. Errores reportados en v0.3.0 (causa y arreglo)
Ninguno sube la RAM de forma medible: son unos cuantos enteros y booleanos más por lista.

- [x] **Espacio y demás teclas de reproducción no funcionan.** Solo las atiende la pantalla "Now playing" (`term/music_player.rs`). En Home, Playlist y Search no hacen nada, y en Search el espacio se escribe en la caja de texto.
  Arreglo: atajos de reproducción globales en el bucle principal (`term/mod.rs`). Search y el campo de link de Home tienen dos estados de foco: escribiendo, o en la lista (se pasa con `↓`/`Tab` y se vuelve con `/` o al escribir). Fuera de la caja de texto, Espacio pausa en cualquier pantalla.
- [x] **La rueda del mouse mueve la selección, no la vista.** `ListItem` y `ListSelector` derivan lo visible de la fila seleccionada, así que hacer scroll también cambia la canción marcada.
  Arreglo: guardar un desplazamiento de vista separado de la selección; la rueda mueve la vista 3 filas y la selección solo cambia con clic o flechas.
- [x] **Las scrollbars no sirven.** Son solo dibujo: el clic y el arrastre sobre ellas no se atienden, y el bucle principal ni siquiera manda los arrastres a las pantallas (`is_down || is_scroll`).
  Arreglo: reenviar los arrastres; clic o arrastre en la columna de la barra salta a esa posición.
- [x] **Un solo clic reproduce.** En las listas el primer clic ya reproduce, y es fácil reproducir algo sin querer. Lo mismo al hacer clic en el título del reproductor (pausa).
  Arreglo: un clic selecciona, doble clic (o Enter) reproduce. Guardar la hora y fila del último clic. Quitar la pausa por clic en el título.
- [x] **La cola no es permanente.** El panel arranca cerrado y no recuerda su estado. Además la cola misma se pierde al cerrar Aria: `last-playlist.json` se lee al iniciar, pero nada lo escribe.
  Arreglo: panel abierto por defecto y estado guardado; guardar la cola y la canción actual al cambiar y al salir, y restaurarlas al iniciar.
- [x] **La cola no se puede recorrer.** Siempre se centra en la canción actual y la rueda sobre ella se ignora.
  Arreglo: el mismo desplazamiento de vista que las listas, volviendo a la canción actual cuando esta cambia.
- [x] **Sin indicación al pasar el mouse en las listas.** Solo el sidebar y los botones reaccionan al hover.
  Arreglo: resaltar la fila bajo el puntero (ya se guarda la posición del mouse).

## 13. Plan de actualización (`aria --update`)
Ya existe: baja el binario `aria-linux-x86_64` del último release, compara la versión con la del programa y reemplaza el ejecutable. Funciona para quien instaló con `install.sh`.

Pasos para publicar la siguiente versión:
1. Subir la versión en `crates/aria/Cargo.toml` (por ejemplo `0.4.0`); `--update` compara contra ella.
2. Commit, push y tag `v0.4.0`; el workflow compila y publica el release.
3. El usuario corre `aria --update` y reinicia Aria.

Mejoras pendientes al actualizador:
- [x] Aceptar también `aria update` (sin guiones).
- [x] AppImage: hoy falla, porque el ejecutable vive en una imagen montada de solo lectura. Usar la ruta de `$APPIMAGE` y bajar el `.AppImage`.
- [x] Verificar que la descarga llegó completa (tamaño del asset) antes de reemplazar.
- [x] Actualizar también `yt-dlp` (`yt-dlp -U`) si fue instalado por el instalador.
- [x] La comparación de versión es por igualdad: si el release fuera más viejo, "actualizaría" hacia atrás. Comparar números.

## 14. v0.4.0 (hecho, sin publicar)
- [x] Portada de la canción en el reproductor (8x4 celdas con medios bloques, solo en terminales de 24 bits). Se guarda en disco solo la de canciones reproducidas: 768 bytes cada una en `~/.cache/aria/covers`.
- [x] Temas: `theme = "red" | "mono" | "light"` en `config.toml`.
- [x] Listas en columnas alineadas (título, artista, duración) con `…`.
- [x] Ecualizador animado en la canción que suena.
- [x] Filas de "cargando" en Home mientras sincroniza; textos de ayuda cuando una lista está vacía.
- [x] Menú con clic derecho o `m`: reproducir, reproducir después, agregar a la cola, quitar, abrir en el navegador, copiar link.
- [x] Barra de atajos según la pantalla; cada atajo se puede clicar.
- [x] Avisos breves en la barra superior.
- [x] Botones de shuffle y repeat (off, all, one).
- [x] Pestaña de letras en el panel derecho (F7).
- [x] Se recuerdan cola, canción, volumen, repeat y panel; la cola restaurada arranca en pausa y no descarga nada hasta dar play.
- [x] Título de la terminal con la canción actual.
- [x] Terminal angosta (<80 columnas): sin sidebar y secciones en la barra superior.
- [x] Ayuda completa con `?`.
- RAM medida en reposo con la biblioteca cargada: 19.4 MB (v0.3.0: 20.7 MB).

## 15. Cuenta y sesión (hecho, sin publicar)
- [x] Sincronización en ambos sentidos con la cuenta: me gusta / quitar me gusta, agregar a una playlist, quitar de una playlist (menú `m` o clic derecho), crear (`n` en Home) y borrar playlists (`Del` dos veces). Probado contra la cuenta real con playlists de prueba que se borraron.
- [x] Lo hecho en el navegador o el celular aparece al abrir la playlist (se vuelve a pedir en segundo plano) y con `F5` en Home (recarga toda la biblioteca).
- [x] La sesión vencida se vuelve a leer del navegador sola; si no se puede, la barra lateral muestra "F8 Sign in".
- [x] Un solo cliente de YouTube Music compartido en vez de uno por búsqueda o letra.
- [x] La paginación ya no mete las "sugerencias" de YouTube dentro de las playlists chicas.
- [x] Se recuerda también la playlist abierta, la pantalla y el segundo de la canción; al dar play sigue desde ahí.
- [x] Portadas de 32x32 dibujadas con cuadrantes (2x2 píxeles por celda) y una portada grande en el panel derecho. Las de 16x16 viejas se reemplazan solas.
- RAM reproduciendo con la biblioteca cargada: 21.8 MB.
