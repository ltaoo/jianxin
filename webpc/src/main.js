import { InboxPageView } from './pages/inbox/index.js';
import { GalleryPageView } from './gallery.js';
// Match the skill's global API convention while using Timeless Lite + DOM.
Object.assign(window, Timeless);
const root = document.getElementById('app');
try {
  const props = { root: root.dataset.root, location: window.location, history: window.history };
  const page = window.location.pathname === `${props.root}/components` ? GalleryPageView(props) : InboxPageView(props);
  root.replaceChildren();
  Timeless.DOM.render(page, root);
} catch (error) {
  console.error(error);
  root.textContent = '邮件界面加载失败，请刷新重试。';
}
