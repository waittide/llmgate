import { reactive } from 'vue';
import { LuFolderTree, LuHome, LuRadio, LuScrollText, LuSettings, LuSparkles } from 'vue-icons-plus/lu';
import type { Component } from 'vue';

// Page 表示应用支持的固定页面集合。
export type Page = 'home' | 'channel' | 'group' | 'model' | 'log' | 'setting';

// NavItem 描述导航按钮使用的页面标识、文案和图标。
export interface NavItem {
    id: Page;
    label: string;
    icon: Component;
}

// NAV_ITEMS 是桌面和移动导航共用的固定导航定义。
export const NAV_ITEMS: NavItem[] = [
    { id: 'home', label: 'Home', icon: LuHome },
    { id: 'channel', label: 'Channel', icon: LuRadio },
    { id: 'group', label: 'Group', icon: LuFolderTree },
    { id: 'model', label: 'Model', icon: LuSparkles },
    { id: 'log', label: 'Log', icon: LuScrollText },
    { id: 'setting', label: 'Setting', icon: LuSettings },
];

const NAV_ORDER: Page[] = NAV_ITEMS.map((item) => item.id); // NAV_ORDER 用于计算页面名称滚动方向。

function getPageFromLocation(): Page | null {
    if (typeof window === 'undefined') return null;
    const hash = window.location.hash.replace(/^#\/?/, '').trim().toLowerCase();
    if (NAV_ORDER.includes(hash as Page)) {
        return hash as Page;
    }
    const path = window.location.pathname.replace(/^\//, '').trim().toLowerCase();
    if (NAV_ORDER.includes(path as Page)) {
        return path as Page;
    }
    return null;
}

// auth-storage 由 zustand persist 写入；这里保持同样的 key/结构，无缝兼容已有会话。
function readPersistedPage(): Page {
    try {
        const raw = localStorage.getItem('llmgate-nav');
        if (raw) {
            const parsed = JSON.parse(raw) as { state?: { currentPage?: Page } };
            const page = parsed?.state?.currentPage;
            if (page && NAV_ORDER.includes(page)) return page;
        }
    } catch {
        // ignore storage parse error
    }
    return getPageFromLocation() || 'home';
}

// appStore 保存应用当前页面及页面名称切换方向（对应 React 版 useAppStore）。
export const appStore = reactive({
    currentPage: readPersistedPage(),
    direction: 0,
});

let persistTimer: number | undefined;
function persistCurrentPage() {
    window.clearTimeout(persistTimer);
    persistTimer = window.setTimeout(() => {
        try {
            localStorage.setItem('llmgate-nav', JSON.stringify({ state: { currentPage: appStore.currentPage } }));
        } catch {
            // ignore storage write error
        }
    }, 0);
}

export function setCurrentPage(page: Page, pushHistory = true) {
    const currentIndex = NAV_ORDER.indexOf(appStore.currentPage);
    const nextIndex = NAV_ORDER.indexOf(page);
    if (typeof window !== 'undefined' && pushHistory) {
        const targetHash = `#${page}`;
        if (window.location.hash !== targetHash) {
            window.history.pushState({ page }, '', targetHash);
        }
    }
    appStore.currentPage = page;
    appStore.direction = nextIndex > currentIndex ? 1 : -1;
    persistCurrentPage();
}

if (typeof window !== 'undefined') {
    window.addEventListener('popstate', () => {
        const page = getPageFromLocation();
        if (page && page !== appStore.currentPage) {
            setCurrentPage(page, false);
        }
    });
}
