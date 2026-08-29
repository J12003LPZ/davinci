import type { Message } from '@pi/core';

export type AgentEvent =
  | { type: 'started'; sessionId: string; timestamp: number }
  | { type: 'message_start'; sessionId: string; messageId: string }
  | { type: 'message_update'; sessionId: string; messageId: string; chunk: string }
  | { type: 'tool_call_start'; sessionId: string; toolCall: { id: string; name: string; arguments: string } }
  | { type: 'tool_execution_end'; sessionId: string; toolCallId: string; result: string }
  | { type: 'turn_end'; sessionId: string }
  | { type: 'completed'; sessionId: string; timestamp: number }
  | { type: 'error'; sessionId: string; error: string };

export interface AgentTool {
  definition(): { name: string; description: string; parameters: unknown };
  execute(args: unknown): Promise<string>;
}

/** TypeScript remains the loop authority. This package documents the event contract the Rust port must match. */
export function transcriptRoles(messages: Message[]): Message['role'][] {
  return messages.map((message) => message.role);
}
