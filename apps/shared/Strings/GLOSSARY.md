# Tend's words

The words in this file mean the same thing on Tend for Mac, the iPhone/iPad
app and the web dashboard. [`glossary.json`](glossary.json) names the key
that shows each word on each surface, and
`crates/tend/tests-js/vocabulary.test.mjs` fails when one of those keys says
something else. A new string for one of these concepts uses the word below.
A new concept that two surfaces show belongs here first. See
`docs/proposals/PARITY.md` §2.2.

## Terms

| Concept | English | Korean | Notes |
|---|---|---|---|
| Pane | pane | **페인** | Never 패인 or 창. "창" is a window. Live Pane is a feature name and stays as it is. |
| Lanes | Your turn · In progress · Finished · Backlog | 내 차례 · 진행 중 · 완료 · 백로그 | |
| Agent at work | Working | 작업 중 | |
| An agent waiting on the person | Needs your decision | 결정이 필요합니다 | For the "now" box on an agent or unit. |
| Decide a pending review | Accept · Return… | 수락 · 돌려보내기… | Push buttons use the same words. The reason is optional. |
| Set a finished unit aside | Looks Good | 확인 완료 | It is not Accept: it marks a finished unit and decides nothing. |
| Give a finished unit more work | Send Back | 다시 시키기 | |
| Close a report | Dismiss | 닫기 | |
| Answer field | Your answer | 답변 | |
| History outcomes | Answered · Returned · Declined · Withdrawn | 답변 완료 · 반려함 · 거절됨 · 철회됨 | |
| Why a question can't be answered | Session ended · Pane gone | 세션이 종료되었습니다 · 페인 없음 | |
| Host states | Online · Offline · Connecting · Degraded · Version skew · Authentication failed · Disabled | 온라인 · 오프라인 · 연결 중 · 성능 저하 · 버전 불일치 · 인증 실패 · 비활성 | Never show the raw state name. |
| Checks tend ran | tend ran N checks: P passed | tend가 검사 N개를 실행했고 P개가 통과했습니다 | |
| Unnamed asker | an agent | 에이전트 | |

## Coming next

These two decisions are not checked yet. PARITY P2 aligns the cards first.

- **Close a live question without answering** is one action, **Decline /
  거절**, on every surface. The reason is optional. The Mac's "Dismiss" on a
  live question becomes Decline.
- A question that can't be answered keeps **Keep / 유지** and **Close all /
  모두 닫기**.
