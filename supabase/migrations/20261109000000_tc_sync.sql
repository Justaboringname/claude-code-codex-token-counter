-- Token Counter cloud sync backend (Supabase / Postgres).
--
-- One row per paired channel, holding only an end-to-end encrypted snapshot.
-- Clients never touch the table directly; they call the tc_* RPCs with a
-- bearer token whose SHA-256 is stored here. Channels are created only by the
-- project owner (SQL editor / service role), so the public API key cannot be
-- used to create rows or fill the database. See docs/SYNC.md.

create schema if not exists private;
revoke all on schema private from public, anon, authenticated;

create table public.tc_channels (
  channel      text primary key check (channel ~ '^[0-9a-f]{32}$'),
  token_hash   text not null check (token_hash ~ '^[0-9a-f]{64}$'),
  label        text,
  version      bigint not null default 0,
  device       text check (device is null or length(device) <= 120),
  generated_at timestamptz,
  payload      text,
  updated_at   timestamptz,
  created_at   timestamptz not null default now()
);
-- RLS on with no policies = no direct table access for anon/authenticated.
alter table public.tc_channels enable row level security;
revoke all on table public.tc_channels from public, anon, authenticated;

create function private.tc_check(p_channel text, p_token text)
returns void
language plpgsql stable security definer set search_path = ''
as $$
begin
  if p_channel is null or p_token is null or not exists (
    select 1 from public.tc_channels c
    where c.channel = p_channel
      and c.token_hash = encode(sha256(convert_to(p_token, 'UTF8')), 'hex')
  ) then
    raise exception 'forbidden' using errcode = '42501';
  end if;
end;
$$;
revoke all on function private.tc_check(text, text) from public, anon, authenticated;

create function public.tc_put(p_channel text, p_token text, p_payload text, p_device text, p_generated_at timestamptz)
returns bigint
language plpgsql volatile security definer set search_path = ''
as $$
declare v bigint;
begin
  perform private.tc_check(p_channel, p_token);
  if p_payload is null or length(p_payload) < 1 or length(p_payload) > 2000000 then
    raise exception 'payload size out of range' using errcode = '22023';
  end if;
  update public.tc_channels
     set payload = p_payload,
         device = left(p_device, 120),
         generated_at = p_generated_at,
         version = version + 1,
         updated_at = now()
   where channel = p_channel
  returning version into v;
  return v;
end;
$$;

create function public.tc_head(p_channel text, p_token text)
returns json
language plpgsql stable security definer set search_path = ''
as $$
declare r json;
begin
  perform private.tc_check(p_channel, p_token);
  select json_build_object(
           'version', c.version,
           'updatedAt', c.updated_at,
           'generatedAt', c.generated_at,
           'device', c.device)
    into r
    from public.tc_channels c where c.channel = p_channel;
  return r;
end;
$$;

create function public.tc_get(p_channel text, p_token text)
returns json
language plpgsql stable security definer set search_path = ''
as $$
declare r json;
begin
  perform private.tc_check(p_channel, p_token);
  select json_build_object(
           'version', c.version,
           'updatedAt', c.updated_at,
           'generatedAt', c.generated_at,
           'device', c.device,
           'payload', c.payload)
    into r
    from public.tc_channels c where c.channel = p_channel;
  return r;
end;
$$;

revoke all on function public.tc_put(text, text, text, text, timestamptz) from public;
revoke all on function public.tc_head(text, text) from public;
revoke all on function public.tc_get(text, text) from public;
grant execute on function public.tc_put(text, text, text, text, timestamptz) to anon, authenticated;
grant execute on function public.tc_head(text, text) to anon, authenticated;
grant execute on function public.tc_get(text, text) to anon, authenticated;
grant usage on schema private to postgres;

-- To register a channel (run as the project owner), with the values printed by
--   node scripts/sync-new-channel.mjs <supabase url> <publishable key>
-- insert into public.tc_channels (channel, token_hash, label)
-- values ('<32 hex>', '<64 hex>', 'my main Mac');
