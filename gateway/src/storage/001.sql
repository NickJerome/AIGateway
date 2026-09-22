CREATE TABLE resources (id TEXT PRIMARY KEY, kind TEXT NOT NULL, physical_name TEXT NOT NULL UNIQUE);
CREATE TABLE plugin_settings (name TEXT PRIMARY KEY, state TEXT NOT NULL, enabled INTEGER NOT NULL,
manifest TEXT NOT NULL, error TEXT, overrides TEXT NOT NULL, installed INTEGER NOT NULL);
