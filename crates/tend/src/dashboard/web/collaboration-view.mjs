/// "42s", "5m", "3h07m", "2d04h"; "—" for nothing.
export function formatDuration(totalSecs) {
  const secs = Math.max(0, Math.floor(totalSecs || 0));
  if (secs === 0) return "—";
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  if (hours < 24) return `${hours}h${String(remMins).padStart(2, "0")}m`;
  const days = Math.floor(hours / 24);
  return `${days}d${String(hours % 24).padStart(2, "0")}h`;
}

export function responseLatency(request) {
  if (!request.reply?.at || !request.created_at) return null;
  const delta = Date.parse(request.reply.at) - Date.parse(request.created_at);
  return Number.isFinite(delta) && delta >= 0 ? delta : null;
}

export function collaborationSummary(requests) {
  const latencies = requests.map(responseLatency).filter(value => value !== null).sort((a, b) => a - b);
  const middle = Math.floor(latencies.length / 2);
  return {
    total: requests.length,
    replies: requests.filter(request => request.reply).length,
    pending: requests.filter(request => ["queued", "claimed"].includes(request.status)).length,
    attention: requests.filter(request => ["blocked", "failed", "expired", "declined"].includes(request.status)).length,
    medianLatency: latencies.length ? (latencies.length % 2 ? latencies[middle] : (latencies[middle - 1] + latencies[middle]) / 2) : null,
  };
}

export function graphFitScale(width, height, viewportWidth, viewportHeight) {
  if (![width, height, viewportWidth, viewportHeight].every(value => Number.isFinite(value) && value > 0)) return 1;
  return Math.max(.1, Math.min(2, (viewportWidth - 16) / width, (viewportHeight - 16) / height));
}

export function participantLocation(participant) {
  return participant?.console ? 'operator console' : participant?.pane || participant?.agent_session_id || 'unknown';
}
