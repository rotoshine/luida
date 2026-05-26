import type { Database } from 'bun:sqlite';
import { AdventurerRepo } from './adventurer';
import { EventRepo } from './event';
import { InmailRepo } from './inmail';
import { QuestRepo } from './quest';
import { RelationshipRepo } from './relationship';

export * from './adventurer';
export * from './event';
export * from './inmail';
export * from './quest';
export * from './relationship';

/** 한 DB 핸들에 대한 모든 repository를 묶은 facade. */
export type Repos = {
  adventurers: AdventurerRepo;
  quests: QuestRepo;
  inmail: InmailRepo;
  events: EventRepo;
  relationships: RelationshipRepo;
  /** 모든 prepared statement를 finalize한다. db.close() 전에 호출 권장. */
  close(): void;
};

export function createRepos(db: Database): Repos {
  const adventurers = new AdventurerRepo(db);
  const quests = new QuestRepo(db);
  const inmail = new InmailRepo(db);
  const events = new EventRepo(db);
  const relationships = new RelationshipRepo(db);
  return {
    adventurers,
    quests,
    inmail,
    events,
    relationships,
    close() {
      adventurers.close();
      quests.close();
      inmail.close();
      events.close();
      relationships.close();
    },
  };
}
