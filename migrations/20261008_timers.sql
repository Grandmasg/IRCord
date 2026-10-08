-- Timers (!timer) delen de tabel met herinneringen; `kind` bepaalt de tekst waarmee ze worden afgeleverd.
ALTER TABLE reminders ADD COLUMN kind TEXT NOT NULL DEFAULT 'remind';
