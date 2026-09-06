import { reactive } from 'vue';

// ToastItem 描述单条通知；variant 决定图标与配色（对齐 sonner 语义）。
export interface ToastItem {
    id: number;
    message: string;
    variant: 'success' | 'error' | 'warning' | 'message';
    duration: number;
}

// toasts 为全局单例队列，由 <Toaster> 渲染。
export const toasts = reactive<ToastItem[]>([]);

let nextId = 1;

function push(message: string, variant: ToastItem['variant'], duration = 4000) {
    const id = nextId++;
    toasts.push({ id, message, variant, duration });
    if (duration > 0) {
        window.setTimeout(() => dismiss(id), duration);
    }
}

export function dismiss(id: number) {
    const index = toasts.findIndex((item) => item.id === id);
    if (index >= 0) toasts.splice(index, 1);
}

// toast 提供与 sonner 一致的命令式 API（本应用仅使用字符串消息形态）。
export const toast = {
    success: (message: string, duration?: number) => push(message, 'success', duration),
    error: (message: string, duration?: number) => push(message, 'error', duration),
    warning: (message: string, duration?: number) => push(message, 'warning', duration),
    message: (message: string, duration?: number) => push(message, 'message', duration),
};
