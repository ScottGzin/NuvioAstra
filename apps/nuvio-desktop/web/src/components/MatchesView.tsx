// Today's fixtures from ESPN with the installed channel whose EPG lists the game.
// A channel is only shown after Rust found the teams in its guide around kick-off.

import type { Channel, LiveMatch, MatchesView as MatchesData } from "../types";
import { t } from "../i18n";

interface Props {
  data: MatchesData | null;
  channels: Channel[];
  loading: boolean;
  onRefresh(): void;
  onTune(channel: Channel): void;
}

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

function TeamBadge({ team }: { team: LiveMatch["home"] }) {
  if (!team) return null;
  return (
    <span className="match-team">
      {team.logo && <img src={team.logo} alt="" loading="lazy" />}
      <span className="match-team-name">{team.short || team.name}</span>
      {team.score != null && <b className="match-score">{team.score}</b>}
    </span>
  );
}

export function Matches({ data, channels, loading, onRefresh, onTune }: Props) {
  const byKey = new Map(channels.map((c) => [c.key, c]));
  const matches = (data?.matches ?? []).filter((m) => m.state === "live" || m.state === "upcoming").slice(0, 12);
  const withChannel = matches.filter((m) => m.channels.length > 0).length;

  return (
    <section className="matches" data-testid="livetv-matches">
      <div className="matches-head">
        <h3>{t("Partidas de hoje")}</h3>
        <span className="muted small">
          {loading ? t("Carregando partidas…") : t("{count} partidas · {found} com canal", { count: matches.length, found: withChannel })}
        </span>
        <button className="chip" onClick={onRefresh} disabled={loading} aria-label={t("Atualizar partidas")}>⟳</button>
      </div>
      {data?.error && <div className="row-error">{data.error}</div>}
      {!loading && matches.length === 0 && <div className="muted small">{t("Nenhuma partida encontrada para hoje.")}</div>}
      <div className="matches-strip">
        {matches.map((match) => (
          <article className={`match-card ${match.state}`} key={match.id} data-testid={`match-${match.id}`}>
            <div className="match-league">
              {match.leagueLogo && <img src={match.leagueLogo} alt="" loading="lazy" />}
              <span>{match.league}</span>
              <span className="match-when">{match.state === "live" ? match.status : clock(match.startMs)}</span>
            </div>
            <div className="match-teams">
              <TeamBadge team={match.home} />
              <span className="match-vs">–</span>
              <TeamBadge team={match.away} />
            </div>
            {match.home == null && <div className="muted small match-event">{match.name}</div>}
            <div className="match-channels">
              {match.channels.length === 0 ? (
                <span className="muted small">{t("Canal não confirmado no guia")}</span>
              ) : (
                match.channels.map((ref) => {
                  const channel = byKey.get(ref.key);
                  return (
                    <button className="chip match-channel" key={ref.key} title={ref.program} onClick={() => channel && onTune(channel)}>
                      {ref.logo && <img src={ref.logo} alt="" loading="lazy" />}
                      {ref.name}
                    </button>
                  );
                })
              )}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
