create table if not exists tables (
  id integer primary key autoincrement,
  fingerprint text not null unique,
  exe text not null,
  game text not null,
  toml text not null,
  bytes integer not null,
  cheats integer not null default 0,
  submitted_by text not null default '',
  built_for text not null default '',
  up integer not null default 0,
  down integer not null default 0,
  downloads integer not null default 0,
  blocked integer not null default 0,
  mirrored integer not null default 0,
  created_at integer not null
);

create index if not exists tables_by_exe on tables (exe, blocked);
create index if not exists tables_to_mirror on tables (mirrored);

create table if not exists votes (
  install text not null,
  table_id integer not null,
  vote integer not null,
  built_for text not null default '',
  created_at integer not null,
  primary key (install, table_id)
);

create table if not exists posts (
  ip text not null,
  at integer not null
);

create index if not exists posts_by_ip on posts (ip, at);

create table if not exists grabs (
  install text not null,
  table_id integer not null,
  created_at integer not null,
  primary key (install, table_id)
);
