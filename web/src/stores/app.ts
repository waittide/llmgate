import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { LucideIcon } from 'lucide-react';
import { Home, Radio, Sparkles, FolderTree, Settings, Logs } from 'lucide-react';

// Page 表示应用支持的固定页面集合。
export type Page = 'home' | 'channel' | 'group' | 'model' | 'log' | 'setting';

// NavItem 描述导航按钮使用的页面标识、文案和图标。
type NavItem = { id: Page; label: string; icon: LucideIcon };

// NAV_ITEMS 是桌面和移动导航共用的固定导航定义。
export const NAV_ITEMS: NavItem[] = [
    { id: 'home', label: 'Home', icon: Home },
    { id: 'channel', label: 'Channel', icon: Radio },
    { id: 'group', label: 'Group', icon: FolderTree },
    { id: 'model', label: 'Model', icon: Sparkles },
    { id: 'log', label: 'Log', icon: Logs },
    { id: 'setting', label: 'Setting', icon: Settings },
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

interface AppState {
    currentPage: Page; // 当前选中的固定页面。
    direction: number; // 页面名称切换时的滚动方向。
    setCurrentPage: (page: Page, pushHistory?: boolean) => void; // 切换当前页面。
}

// useAppStore 保存应用当前页面及页面名称切换方向。
export const useAppStore = create<AppState>()(
    persist(
        (set, get) => ({
            currentPage: getPageFromLocation() || 'home',
            direction: 0,
            setCurrentPage: (page, pushHistory = true) => {
                const currentIndex = NAV_ORDER.indexOf(get().currentPage);
                const nextIndex = NAV_ORDER.indexOf(page);
                if (typeof window !== 'undefined' && pushHistory) {
                    const targetHash = `#${page}`;
                    if (window.location.hash !== targetHash) {
                        window.history.pushState({ page }, '', targetHash);
                    }
                }
                set({ currentPage: page, direction: nextIndex > currentIndex ? 1 : -1 });
            },
        }),
        {
            name: 'nav-storage',
        }
    )
);

if (typeof window !== 'undefined') {
    window.addEventListener('popstate', () => {
        const page = getPageFromLocation();
        if (page && page !== useAppStore.getState().currentPage) {
            useAppStore.getState().setCurrentPage(page, false);
        }
    });
}
