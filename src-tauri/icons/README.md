# Ikon Aplikasi

Tauri **wajib** memiliki set ikon sebelum `tauri dev` / `tauri build` dijalankan.
Generate otomatis dari satu file PNG (ukuran minimal 1024x1024px):

```bash
cd launcher
npx tauri icon path/ke/logo-anda.png
```

Perintah di atas akan menghasilkan `src-tauri/icons/icon.ico`, `icon.png`,
`icon.ico`, dan varian lainnya yang dibutuhkan `tauri.conf.json`.
