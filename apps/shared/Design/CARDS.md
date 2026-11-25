# Decision cards: one anatomy, one action set

Tend for Mac, the iPhone/iPad app and the web dashboard draw a decision in
Your turn the same way. A decision is a question an agent asked the person,
or a result waiting for review. Each platform keeps its own controls and
placement: buttons, menus, sheets, keyboard shortcuts, Face ID. The parts
come in the same order and the actions use the same words. Words are from
`../Strings/GLOSSARY.md`. Plan: `docs/proposals/PARITY.md` P2.

## Anatomy, top to bottom

1. **Kind row:** the kind's icon and word ("Question" / "질문", "Review" /
   "리뷰"), in the attention ink, and "Sending…" while a decision is
   in flight. The overflow (⋯) sits at the right end. The Mac also keeps
   its context menu.
2. **Title:** the decision needed, or the question's first line. At most 4
   lines on the Mac and the web, 6 on iOS (Dynamic Type).
3. **Inset box**, in this order, with each part only when present:
   tend's own checks, the detail, the recommendation (💡), the evidence
   chips, the numbered option rows, then the **action row or the composer**.
4. **Failure line**, when the last attempt failed. It says why, and nothing
   was decided.
5. **Meta row:** asker · host · project, with the relative time at the right
   end. The host is always shown, as on work-unit cards. On iOS the time
   moves under the meta on a narrow width.

**Attention mark:** a Your-turn card has a stripe at its leading edge and a tinted fill.
The stripe is `stripe-width` (`TendDesignTokens.Metric.stripeWidth`) in the
orange state colour. The fill is `card-your-turn` (`TendDesignTokens.cardYourTurn`).
The same mark goes on a Your-turn work unit card.

## Actions

The same buttons appear in the same order with the same words. "Primary" is
the platform's prominent style.

### Question (`human_request`)

| Where | Without options | With options |
|---|---|---|
| Action row | **Answer…** · **Decline…** | option rows, then **Other…** · **Decline…** |
| Answer composer | field "Your answer" / "답변"; **Cancel** · **Send** (primary, needs text) | an option fills the field |
| Decline composer | field "Reason (optional)" / "이유 (선택)"; **Cancel** · **Decline** (primary, enabled without text) | |
| ⋯ | Open Agent (where the surface can) | |

- **Decline / 거절** is the one way to close a live question without
  answering it. The Mac's "Dismiss" on a live question becomes Decline. An
  empty reason is fine: tendd sends the asker its standard "declined
  without an answer" text.
- "Dismiss" stays for closing a *report*, such as Close all's result.
- A question that can't be answered keeps **Keep / 유지** and **Close all
  / 모두 닫기**, in its own group.

### Review (`review`)

| Where | |
|---|---|
| Action row | option rows (when the asker offered options), then **Accept** · **Return…** |
| An option chosen | composer with the option shown; **Cancel** · **Accept** (primary) |
| Return composer | field "Note for the asker (optional)" / "요청자에게 남길 메모 (선택)"; **Cancel** · **Return** (primary, enabled without text) |
| ⋯ | **Withdraw** / 철회 |

### Another host's decision this surface cannot decide

The action row is replaced by one line: "Decide this on %@." / "%@에서
결정하세요." The Mac adds "Open on the Dashboard" when it knows the
dashboard. An observe-only host is described as observe-only, using the
same sentence.

## Not here

- Work-unit cards are already alike. Only their attention mark follows this page.
- Counts and badges are covered by PARITY P3.
- Exact type sizes and radii are covered by PARITY P5.
