import { Button, Glyph } from '../../../packages/jxui/index.js';
import { SyncLogsModel } from './index.model.js';

function timeLabel(timestamp) {
  return timestamp == null ? '尚无同步记录' : new Date(timestamp * 1000).toLocaleString('zh-CN', { hour12: false });
}

export function SyncLogsView(props) {
  const model = SyncLogsModel(props), state = ref(model.state), derived = [];
  const off = model.onStateChange(s => state.as(s));
  const pick = fn => { const value = Timeless.computed(state, fn); derived.push(value); return value; };
  return View({ as: 'main', class: 'jx-sync-logs', attributes: { n: 'sync-logs-page' },
    onMounted() { model.start(); },
    onUnmounted() { off(); derived.forEach(value => value.destroy()); state.destroy(); model.destroy(); },
  }, [
    View({ as: 'header', class: 'jx-row jx-sync-logs-header', attributes: { n: 'sync-logs-header' } }, [
      View({ as: 'a', class: 'jx-button jx-button--ghost', attributes: { n: 'sync-logs-back', href: `${props.root}/` } }, [Glyph('back'), '返回邮件']),
      View({ as: 'h1', attributes: { n: 'sync-logs-title' } }, ['同步日志']),
      View({ class: 'jx-spacer', attributes: { n: 'sync-logs-header-spacer' } }),
      Button({ store: model.ui.refresh, icon: 'refresh', attributes: { n: 'sync-logs-refresh' } }),
    ]),
    View({ as: 'p', class: 'jx-muted', attributes: { n: 'sync-logs-description' } }, [pick(s => `记录每次同步结束的时间和结果，每个邮箱保留最近 ${s.retention} 条。首页每 5 秒自动更新。`)]),
    View({ class: 'jx-sync-accounts', attributes: { n: 'sync-logs-mailboxes' } }, [For({ each: pick(s => s.accounts), render: account =>
      View({ as: 'article', class: 'jx-sync-account', attributes: { n: `sync-log-mailbox-${account.id}`, 'aria-label': account.email } }, [
        View({ as: 'strong', attributes: { n: 'sync-log-mailbox-email' } }, [account.email]),
        View({ as: 'span', class: `jx-sync-result is-${account.status.tone}`, attributes: { n: 'sync-log-mailbox-status' } }, [account.status.label]),
        View({ as: 'span', class: 'jx-muted', attributes: { n: 'sync-log-mailbox-last-time' } }, [`最近同步：${timeLabel(account.last_sync_at)}`]),
        account.last_result ? View({ as: 'span', class: 'jx-muted', attributes: { n: 'sync-log-mailbox-last-result' } }, [`最近结果：${account.last_result.label}`]) : undefined,
      ]) }),
    ]),
    Show({ when: pick(s => s.loaded && !s.accounts.length), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'sync-logs-no-accounts' } }, ['尚未添加邮箱，请返回邮件页面添加。']) }),
    View({ class: 'jx-row jx-sync-logs-filter', attributes: { n: 'sync-logs-filter' } }, [
      View({ as: 'label', attributes: { n: 'sync-logs-filter-label', for: 'sync-logs-account' } }, ['邮箱']),
      View({ as: 'select', class: 'jx-input', attributes: { n: 'sync-logs-account-filter', id: 'sync-logs-account', 'aria-label': '筛选邮箱' },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { target.value = model.state.account; };
          const change = () => model.select(target.value);
          apply(); const stop = model.onStateChange(apply); target.addEventListener('change', change);
          return () => { stop(); target.removeEventListener('change', change); };
        },
      }, [View({ as: 'option', attributes: { n: 'sync-logs-all-accounts', value: '' } }, ['全部邮箱']),
        For({ each: pick(s => s.accounts), render: account => View({ as: 'option', attributes: { n: `sync-logs-filter-${account.id}`, value: account.id } }, [account.email]) })]),
    ]),
    Show({ when: pick(s => s.error), ok: () => View({ as: 'p', class: 'jx-import-error', attributes: { n: 'sync-logs-error', role: 'alert' } }, [pick(s => s.error)]) }),
    View({ as: 'p', class: 'jx-muted', attributes: { n: 'sync-logs-load-status', role: 'status', 'aria-live': 'polite' } }, [pick(s => s.loading ? '正在读取同步日志…' : '')]),
    View({ class: 'jx-sync-table-scroll', attributes: { n: 'sync-logs-table-scroll', tabindex: '0', 'aria-label': '同步历史' } }, [
      View({ as: 'table', class: 'jx-sync-table', attributes: { n: 'sync-logs-table' } }, [
        View({ as: 'caption', attributes: { n: 'sync-logs-table-caption' } }, ['同步历史 · 最新记录在前']),
        View({ as: 'thead', attributes: { n: 'sync-logs-table-head' } }, [View({ as: 'tr', attributes: { n: 'sync-logs-column-headings' } },
          [['time', '完成时间'], ['mailbox', '邮箱'], ['result', '结果']].map(([name, title]) => View({ as: 'th', attributes: { n: `sync-logs-heading-${name}`, scope: 'col' } }, [title])))]),
        View({ as: 'tbody', attributes: { n: 'sync-logs-table-body' } }, [For({ each: pick(s => s.logs), render: entry => View({ as: 'tr', attributes: { n: `sync-log-entry-${entry.id}` } }, [
          View({ as: 'td', attributes: { n: 'sync-log-entry-time' } }, [timeLabel(entry.completed_at)]),
          View({ as: 'td', attributes: { n: 'sync-log-entry-email' } }, [entry.email]),
          View({ as: 'td', attributes: { n: 'sync-log-entry-result' } }, [View({ as: 'span', class: `jx-sync-result is-${entry.result.tone}`, attributes: { n: 'sync-log-entry-result-label' } }, [entry.result.state === 'synced' ? '同步成功' : entry.result.label])]),
        ]) })]),
      ]),
    ]),
    Show({ when: pick(s => s.loaded && !s.loading && !s.error && !s.logs.length), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'sync-logs-empty' } }, ['暂无同步记录；邮箱完成同步后会显示在这里。']) }),
    View({ class: 'jx-row jx-sync-logs-pagination', attributes: { n: 'sync-logs-pagination' } }, [
      Button({ store: model.ui.prev, attributes: { n: 'sync-logs-prev' } }),
      View({ as: 'span', class: 'jx-muted', attributes: { n: 'sync-logs-page-number' } }, [pick(s => `第 ${s.page + 1} 页`)]),
      Button({ store: model.ui.next, attributes: { n: 'sync-logs-next' } }),
    ]),
  ]);
}
