import { onBeforeUnmount, onMounted, type Ref } from 'vue';

// useClickOutside 在指针按下目标元素外部时触发回调（对应 React 版 useClickOutside）。
export function useClickOutside(target: Ref<HTMLElement | null>, onOutside: () => void) {
    const handler = (event: PointerEvent) => {
        const el = target.value;
        if (el && event.target instanceof Node && !el.contains(event.target)) {
            onOutside();
        }
    };

    onMounted(() => document.addEventListener('pointerdown', handler, true));
    onBeforeUnmount(() => document.removeEventListener('pointerdown', handler, true));
}
