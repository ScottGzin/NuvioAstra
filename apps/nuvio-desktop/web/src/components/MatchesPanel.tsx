// Live TV → "Jogos de hoje": today's games and scores (ESPN). "Onde passa" reads, only for the game
// clicked, the guides of the sports channels and lists the ones showing it; a click tunes in.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { Channel, MatchChannel, SportsMatch } from "../types";
import { locale, t } from "../i18n";

const REFRESH_MS = 2 * 60 * 1000;

function TeamLine({ team }: { team: SportsMatch["home"] }) {
  if (!team) return null;
  return (
    <div className="match-team">
      {team.logo ? <img src={team.logo} alt="" loading="lazy" /> : <span className="match-team-dot" aria-hidden />}
      <span className="match-team-name">{team.name}</span>
      {team.score != null && <span className="match-score">{team.score}</span>}
    </div>
  );
}

export function MatchesPanel({ channels, onTune }: { channels: Channel[]; onTune(channel: Channel): void }) {
  const [matches, setMatches] = useState<SportsMatch[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [found, setFound] = useState<Record<string, MatchChannel[] | "loading" | string>>({});

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api.liveTvMatches(new Date().getTimezoneOffset()).then(
        (v) => {
          if (cancelled) return;
          setMatches(v.matches);
          setError(v.error);
        },
        (e) => !cancelled && setError(errorText(e)),
      );
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const whereToWatch = async (m: SportsMatch) => {
    setOpen(open === m.id ? null : m.id);
    if (found[m.id] && found[m.id] !== "loading" && typeof found[m.id] !== "string") return;
    setFound((f) => ({ ...f, [m.id]: "loading" }));
    try {
      const list = await api.liveTvMatchChannels(m.id, channels.map((c) => ({ key: c.key, name: c.name, genres: c.genres, logo: c.logo })), new Date().getTimezoneOffset());
      setFound((f) => ({ ...f, [m.id]: list }));
    } catch (e) {
      setFound((f) => ({ ...f, [m.id]: errorText(e) }));
    }
  };

  const time = (ms: number) => new Date(ms).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });

  return (
    <div className="matches" data-testid="matches">
      <div className="channel-list-title">
        <span>{t("Jogos de hoje")}</span>
        <span className="muted small">{matches ? (matches.length === 1 ? t("1 jogo") : t("{n} jogos", { n: matches.length })) : ""}</span>
      </div>
      {!matches && !error && <div className="screen-message">{t("Carregando jogos…")}</div>}
      {error && <div className="muted small pad">{t(error)}</div>}
      {matches?.length === 0 && <div className="screen-message muted">{t("Nenhum jogo hoje nas ligas acompanhadas.")}</div>}
      {matches?.map((m) => {
        const result = found[m.id];
        return (
          <div key={m.id} className={`match-card match-${m.state} ${open === m.id ? "open" : ""}`} data-match={m.id}>
            <button className="match-main" onClick={() => void whereToWatch(m)} title={t("Onde passa")}>
              <div className="match-head">
                {m.leagueLogo && <img className="match-league-logo" src={m.leagueLogo} alt="" />}
                <span className="match-league">{m.league}</span>
                <span className="spacer" />
                <span className={`match-status ${m.state}`}>{m.state === "upcoming" ? time(m.startMs) : m.state === "live" ? `● ${m.status}` : m.status}</span>
              </div>
              {m.home || m.away ? (
                <>
                  <TeamLine team={m.home} />
                  <TeamLine team={m.away} />
                </>
              ) : (
                <div className="match-team"><span className="match-team-name">{m.name}</span></div>
              )}
            </button>
            {open === m.id && (
              <div className="match-channels" data-testid="match-channels">
                {result === "loading" && <div className="muted small"><span className="spinner" aria-hidden /> {t("Procurando nos guias dos canais de esporte…")}</div>}
                {typeof result === "string" && result !== "loading" && <div className="muted small">{t(result)}</div>}
                {Array.isArray(result) && result.length === 0 && <div className="muted small">{t("Nenhum canal com guia mostra este jogo.")}</div>}
                {Array.isArray(result) &&
                  result.map((c) => {
                    const channel = channels.find((x) => x.key === c.key);
                    return (
                      <button key={c.key} className="match-channel" onClick={() => channel && onTune(channel)} disabled={!channel} title={c.program}>
                        {c.logo ? <img src={c.logo} alt="" /> : <span className="match-channel-dot" aria-hidden>{c.name.slice(0, 2)}</span>}
                        <span>{c.name}</span>
                        <span className="muted small">{c.program}</span>
                      </button>
                    );
                  })}
              </div>
            )}
          </div>
        );
      })}
      <div className="muted small pad">{t("Placares: ESPN.")}</div>
    </div>
  );
}
