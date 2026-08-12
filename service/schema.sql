create table if not exists tables (
  id integer primary key autoincrement,
  fingerprint text not null unique,
  exe text not null,
  game text not null,
  toml text not null,
  bytes integer not null,
  cheats integer not null default 0,
  submitted_by text not null default '',
  -- whoever worked the addresses out. free text off the table itself, and
  -- nothing checks it, because it says where a table came from rather than
  -- who is standing behind it. submitted_by is the one tied to a key
  author text not null default '',
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
-- searching by name rather than by executable, for when we picked the wrong
-- binary or the table was written against a different edition
create index if not exists tables_by_game on tables (game, blocked);

-- one exe's table reported working for another exe's game.
--
-- a game can ship more than one binary and no rule picks the right one every
-- time: fallout has falloutw.exe and falloutwHR.exe a kilobyte apart, and the
-- tables all name the first. somebody who searches, finds the table and
-- upvotes it while running the other one has proved they belong together, so
-- the next person gets it without searching.
--
-- keyed on the install so one person counts once, however many times they
-- vote. an alias only shows up for other people once `alias_floor` of them
-- agree, because one voter should not get to redirect a name for everybody.
create table if not exists aliases (
  from_exe text not null,
  to_exe text not null,
  install text not null,
  created_at integer not null,
  primary key (from_exe, to_exe, install)
);

create index if not exists aliases_by_target on aliases (to_exe, from_exe);

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

-- rate limiting for everything that is not a submission. `what` is there so
-- one address cannot mint a hundred install ids and vote once from each
create table if not exists hits (
  ip text not null,
  kind text not null,
  what text not null default '',
  at integer not null
);

create index if not exists hits_by_ip on hits (kind, ip, at);

create table if not exists grabs (
  install text not null,
  table_id integer not null,
  created_at integer not null,
  primary key (install, table_id)
);

create table if not exists accounts (
  name text primary key,
  pubkey text not null unique,
  created_at integer not null
);
