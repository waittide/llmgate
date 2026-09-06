// MorphingDialog 的共享类型与注入键（<script setup> 不能含模块导出，故独立成文件）。
import type { InjectionKey, Ref } from 'vue';
import type { Transition } from 'motion-v';

export type MorphingDialogContextType = {
  isOpen: Ref<boolean>;
  setIsOpen: (value: boolean | ((prev: boolean) => boolean)) => void;
  uniqueId: string;
  triggerRef: Ref<HTMLElement | null>;
};

export const MORPHING_DIALOG_KEY: InjectionKey<MorphingDialogContextType> = Symbol('MorphingDialogContext');

export interface MorphingDialogProps {
  isOpen?: boolean;
  open?: boolean;
  modelValue?: boolean;
  transition?: Transition;
  defaultOpen?: boolean;
}
