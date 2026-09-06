// 桶导出：与 React 版 '@/components/ui/morphing-dialog' 的引用路径保持一致。
import { inject } from 'vue';
import { MORPHING_DIALOG_KEY, type MorphingDialogContextType } from './morphing-dialog-context';

export { default as MorphingDialog } from './MorphingDialog.vue';
export { default as MorphingDialogTrigger } from './MorphingDialogTrigger.vue';
export { default as MorphingDialogContainer } from './MorphingDialogContainer.vue';
export { default as MorphingDialogContent } from './MorphingDialogContent.vue';
export { default as MorphingDialogClose } from './MorphingDialogClose.vue';
export { default as MorphingDialogTitle } from './MorphingDialogTitle.vue';
export { default as MorphingDialogSubtitle } from './MorphingDialogSubtitle.vue';
export { default as MorphingDialogDescription } from './MorphingDialogDescription.vue';
export { default as MorphingDialogImage } from './MorphingDialogImage.vue';
export type { MorphingDialogContextType, MorphingDialogProps } from './morphing-dialog-context';
export { MORPHING_DIALOG_KEY } from './morphing-dialog-context';

// useMorphingDialog 取出 MorphingDialogProvider 注入的上下文（子组件内部使用）。
export function useMorphingDialog(): MorphingDialogContextType {
    const ctx = inject(MORPHING_DIALOG_KEY);
    if (!ctx) {
        throw new Error('useMorphingDialog 必须在 <MorphingDialog> 内部使用');
    }
    return ctx;
}
