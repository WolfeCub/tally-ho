CREATE TABLE "statements" (
    "id" BLOB NOT NULL,
    "label" TEXT NOT NULL,
    "currency" TEXT NOT NULL,
    "begins_on" TEXT NOT NULL,
    "ends_on" TEXT NOT NULL,
    "imported_at" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "charges" (
    "id" BLOB NOT NULL,
    "statement_id" BLOB NOT NULL,
    "charged_on" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "amount" TEXT NOT NULL,
    "position" BIGINT NOT NULL,
    "receipt_id" BLOB,
    "confirmed" BOOLEAN NOT NULL,
    "no_receipt" BOOLEAN NOT NULL,
    "person_id" BLOB,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_charges_by_statement_id" ON "charges" ("statement_id");
-- #[toasty::breakpoint]
CREATE INDEX "index_charges_by_receipt_id" ON "charges" ("receipt_id");
-- #[toasty::breakpoint]
CREATE INDEX "index_charges_by_person_id" ON "charges" ("person_id");
