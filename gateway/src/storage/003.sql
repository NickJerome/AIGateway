CREATE TABLE usage(id INTEGER PRIMARY KEY,plugin TEXT NOT NULL,request TEXT NOT NULL,time INTEGER NOT NULL,value TEXT NOT NULL);
CREATE INDEX usage_plugin_time ON usage(plugin,time);
CREATE TABLE logs(id INTEGER PRIMARY KEY,plugin TEXT NOT NULL,request TEXT NOT NULL,time INTEGER NOT NULL,value TEXT NOT NULL);
CREATE INDEX logs_plugin_time ON logs(plugin,time);
CREATE TABLE traces(plugin TEXT NOT NULL,request TEXT NOT NULL,time INTEGER NOT NULL,value TEXT NOT NULL,active INTEGER NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(plugin,request));
CREATE TABLE trace_chunks(plugin TEXT NOT NULL,request TEXT NOT NULL,side TEXT NOT NULL,seq INTEGER NOT NULL,body BLOB NOT NULL,PRIMARY KEY(plugin,request,side,seq));
