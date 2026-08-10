ALTER TABLE "line_items" ADD COLUMN "person_id" BLOB;
-- #[toasty::breakpoint]
CREATE INDEX "index_line_items_by_person_id" ON "line_items" ("person_id");
-- #[toasty::breakpoint]
CREATE TABLE "people" (
    "id" BLOB NOT NULL,
    "name" TEXT NOT NULL,
    "description" TEXT,
    "created_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
