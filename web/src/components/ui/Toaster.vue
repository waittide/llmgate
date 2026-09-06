<script setup lang="ts">
import { computed, type HTMLAttributes } from 'vue';
import { LuCheck, LuX, LuAlertTriangle, LuInfo } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';
import { toasts, dismiss, type ToastItem } from '@/composables/toast';

export type ToastPosition =
    | 'top-left'
    | 'top-right'
    | 'top-center'
    | 'bottom-left'
    | 'bottom-right'
    | 'bottom-center';

const props = withDefaults(
    defineProps<{
        position?: ToastPosition;
        richColors?: boolean;
        expand?: boolean;
        duration?: number;
        visibleToasts?: number;
        closeButton?: boolean;
        theme?: string;
        class?: HTMLAttributes['class'];
    }>(),
    {
        position: 'top-left',
        richColors: false,
        expand: false,
        duration: 4000,
        visibleToasts: 5,
        closeButton: true,
        theme: 'system',
        class: undefined,
    }
);

const positionClasses = computed(() => {
    switch (props.position) {
        case 'top-left':
            return 'top-4 left-4 items-start';
        case 'top-right':
            return 'top-4 right-4 items-end';
        case 'top-center':
            return 'top-4 left-1/2 -translate-x-1/2 items-center';
        case 'bottom-left':
            return 'bottom-4 left-4 items-start';
        case 'bottom-right':
            return 'bottom-4 right-4 items-end';
        case 'bottom-center':
            return 'bottom-4 left-1/2 -translate-x-1/2 items-center';
        default:
            return 'top-4 left-4 items-start';
    }
});
</script>

<template>
    <Teleport to="body">
        <section
            aria-label="Notifications"
            tabindex="-1"
            data-slot="toaster"
            data-position="props.position"
            :class="
                cn(
                    'fixed z-[100] flex flex-col gap-2 pointer-events-none w-auto max-w-[calc(100vw-32px)]',
                    positionClasses,
                    props.class
                )
            "
        >
            <TransitionGroup
                enter-active-class="transition duration-200 ease-out"
                enter-from-class="opacity-0 -translate-y-2 scale-95"
                enter-to-class="opacity-100 translate-y-0 scale-100"
                leave-active-class="transition duration-150 ease-in pointer-events-none"
                leave-from-class="opacity-100 scale-100"
                leave-to-class="opacity-0 scale-95"
                move-class="transition-all duration-200 ease-out"
            >
                <div
                    v-for="item in toasts"
                    :key="item.id"
                    role="status"
                    aria-live="polite"
                    data-slot="toast"
                    :data-type="item.variant"
                    class="pointer-events-auto relative flex items-center gap-3 w-full max-w-[356px] min-w-[280px] p-4 rounded-xl border border-border bg-popover text-popover-foreground shadow-lg text-sm font-medium transition-all"
                >
                    <!-- Variant Icon -->
                    <div class="flex items-center justify-center shrink-0">
                        <LuCheck v-if="item.variant === 'success'" class="size-4 text-emerald-500" />
                        <LuX v-else-if="item.variant === 'error'" class="size-4 text-destructive" />
                        <LuAlertTriangle v-else-if="item.variant === 'warning'" class="size-4 text-amber-500" />
                        <LuInfo v-else class="size-4 text-muted-foreground" />
                    </div>

                    <!-- Toast Message -->
                    <div class="flex-1 min-w-0 text-sm break-words leading-snug">
                        {{ item.message }}
                    </div>

                    <!-- Close Button -->
                    <button
                        v-if="closeButton"
                        type="button"
                        aria-label="Close notification"
                        class="text-muted-foreground hover:text-foreground p-1 rounded-md hover:bg-muted transition-colors shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        @click="dismiss(item.id)"
                    >
                        <LuX class="size-3.5" />
                    </button>
                </div>
            </TransitionGroup>
        </section>
    </Teleport>
</template>
