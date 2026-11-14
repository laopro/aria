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
