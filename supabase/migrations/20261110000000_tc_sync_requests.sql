-- Token Counter cloud sync: refresh requests and uploader liveness.
--
-- 1. A viewer (phone, another Mac) can ask the uploading Mac for a fresh
--    snapshot now instead of waiting for its next scheduled upload:
--    tc_request stamps the channel, the uploader sees it on its next tc_poll
--    (every few seconds) and uploads.
-- 2. A second Mac can be a standby uploader: it takes over while the main Mac
--    is asleep or offline. For that the main Mac's polls record that it is
--    alive (primary_seen_at), and whoever is uploading records uploader_seen_at
--    so a viewer knows whether anyone will answer its request.
--
-- Only timestamps are stored; the payload stays end-to-end encrypted. Ages are
-- computed here, on the server's clock, so no client depends on its own.
-- Older clients keep working: tc_head / tc_get / tc_put are unchanged.

alter table public.tc_channels
  add column refresh_requested_at timestamptz,
  add column primary_seen_at timestamptz,
  add column uploader_seen_at timestamptz;

-- A viewer asks for a fresh upload. Returns when it asked (compare with a
-- later head's updatedAt) and how long ago an uploader was last seen
-- (null = never: nobody is listening, don't wait).
create function public.tc_request(p_channel text, p_token text)
returns json
language plpgsql volatile security definer set search_path = ''
as $$
declare r json;
begin
  perform private.tc_check(p_channel, p_token);
  update public.tc_channels c
     set refresh_requested_at = now()
   where c.channel = p_channel
  returning json_build_object(
           'requestedAt', c.refresh_requested_at,
           'uploaderAgeSecs', extract(epoch from now() - c.uploader_seen_at))
    into r;
  return r;
end;
$$;

-- An uploading Mac's poll. p_role: 'primary' (the main Mac), 'standby-active'
-- (a standby that has taken over) or 'standby' (a standby that is only
-- watching). The first two mark an uploader as present, 'primary' also marks
-- the main Mac as alive; both stamps are written at most every 20 s.
create function public.tc_poll(p_channel text, p_token text, p_role text)
returns json
language plpgsql volatile security definer set search_path = ''
as $$
declare r json;
begin
  perform private.tc_check(p_channel, p_token);
  if p_role = 'primary' then
    update public.tc_channels c
       set primary_seen_at = now(), uploader_seen_at = now()
     where c.channel = p_channel
       and (c.primary_seen_at is null or c.primary_seen_at < now() - interval '20 seconds'
            or c.uploader_seen_at is null or c.uploader_seen_at < now() - interval '20 seconds');
  elsif p_role = 'standby-active' then
    update public.tc_channels c
       set uploader_seen_at = now()
     where c.channel = p_channel
       and (c.uploader_seen_at is null or c.uploader_seen_at < now() - interval '20 seconds');
  end if;
  select json_build_object(
           'version', c.version,
           'device', c.device,
           'updatedAgeSecs', extract(epoch from now() - c.updated_at),
           'requestedAt', c.refresh_requested_at,
           'requestAgeSecs', extract(epoch from now() - c.refresh_requested_at),
           'primaryAgeSecs', extract(epoch from now() - c.primary_seen_at))
    into r
    from public.tc_channels c where c.channel = p_channel;
  return r;
end;
$$;

revoke all on function public.tc_request(text, text) from public;
revoke all on function public.tc_poll(text, text, text) from public;
grant execute on function public.tc_request(text, text) to anon, authenticated;
grant execute on function public.tc_poll(text, text, text) to anon, authenticated;
