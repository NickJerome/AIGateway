CREATE TABLE do_migrations(plugin TEXT NOT NULL,tag TEXT NOT NULL,position INTEGER NOT NULL,classes TEXT NOT NULL,PRIMARY KEY(plugin,tag),UNIQUE(plugin,position));
CREATE TABLE do_values(plugin TEXT NOT NULL,object TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(plugin,object,key));
