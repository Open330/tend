// Match tmux's numeric client-view suffix without hiding ordinary names.
export function baseSessionName(value) {
  let name = String(value || "");
  while (/^.+~view~[0-9]+$/.test(name)) name = name.replace(/~view~[0-9]+$/, "");
  return name;
}
