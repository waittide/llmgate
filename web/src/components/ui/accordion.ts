import type { ComputedRef, InjectionKey } from "vue"

import Accordion from "./accordion.vue"
import AccordionItem from "./accordion-item.vue"
import AccordionHeader from "./accordion-header.vue"
import AccordionTrigger from "./accordion-trigger.vue"
import AccordionContent from "./accordion-content.vue"

export interface AccordionContext {
  isOpen: (value: string) => boolean
  toggleItem: (value: string) => void
  disabled: ComputedRef<boolean>
}

export interface AccordionItemContext {
  value: string
  isOpen: ComputedRef<boolean>
  disabled: ComputedRef<boolean>
  toggle: () => void
}

export const ACCORDION_INJECTION_KEY: InjectionKey<AccordionContext> =
  Symbol("AccordionContext")

export const ACCORDION_ITEM_INJECTION_KEY: InjectionKey<AccordionItemContext> =
  Symbol("AccordionItemContext")

// AccordionPrimitive 兼容 Radix 原语风格调用。
export const AccordionPrimitive = {
  Root: Accordion,
  Item: AccordionItem,
  Header: AccordionHeader,
  Trigger: AccordionTrigger,
  Content: AccordionContent,
}

export {
  Accordion,
  Accordion as default,
  AccordionItem,
  AccordionHeader,
  AccordionTrigger,
  AccordionContent,
}
