---------------------- MODULE main -------------------------
EXTENDS Naturals, TLC, FiniteSets
CONSTANTS SPSC_SIZE,
          URING_SIZE,
          NUM_OPS

(*
--algorithm KVell {
    variables
      spsc = [i \in 0..SPSC_SIZE-1 |-> [type |-> "read", token |-> 0, key |-> 0]],
      spsc_head = 0,
      spsc_tail = 0,
      
      sq = [i \in 0..URING_SIZE-1 |-> 0],
      sq_head = 0,
      sq_tail = 0,
      
      cq = [i \in 0..URING_SIZE-1 |-> 0],
      cq_head = 0,
      cq_tail = 0,
      
      in_flight = {},
      retrieved = {},
      next_token = 1,
      produced = 0;

    define {
        SPSCEmpty == spsc_head = spsc_tail
        SPSCFull  == (spsc_tail + 1) % SPSC_SIZE = spsc_head
        SQFull    == (sq_tail + 1) % URING_SIZE = sq_head
        CQFull    == (cq_tail + 1) % URING_SIZE = cq_head
        CQEmpty   == cq_head = cq_tail
    };
    
    procedure SPSCEnqueue(val)
    {
      AwaitSpace:
        await ~SPSCFull;
      SPSCEnq:
        spsc[spsc_tail % SPSC_SIZE] := val;
        spsc_tail := spsc_tail + 1;
    }

    procedure SPSCDequeue()
      variables ret = [type |-> "read", token |-> 0, key |-> 0];
    {
      AwaitResult:
        await ~SPSCEmpty;
      SPSCDeq:
        ret := spsc[spsc_head % SPSC_SIZE];
        spsc_head := spsc_head + 1;
    }

    procedure SQEnqueue(val)
    {
      AwaitSpace:
        await ~SQFull;
      SQEnq:
        sq[sq_tail % URING_SIZE] := val;
        sq_tail := sq_tail + 1;
        return;
    }

    process (Producer = "producer")
    {
      Produce:
        while (produced < NUM_OPS) {
          AwaitSpace:
            await ~SPSCFull;
          Push:
            spsc[spsc_tail % SPSC_SIZE] := [type |-> "read", token |-> next_token, key |-> 0];
            spsc_tail  := spsc_tail + 1;
            next_token := next_token + 1;
            produced   := produced + 1;
        }
    }
}
*)
\* BEGIN TRANSLATION (chksum(pcal) = "9cea67ee" /\ chksum(tla) = "5990d0eb")
\* Label AwaitSpace of procedure SPSCEnqueue at line 35 col 9 changed to AwaitSpace_
\* Label AwaitSpace of procedure SQEnqueue at line 54 col 9 changed to AwaitSpace_S
\* Parameter val of procedure SPSCEnqueue at line 32 col 27 changed to val_
CONSTANT defaultInitValue
VARIABLES spsc, spsc_head, spsc_tail, sq, sq_head, sq_tail, cq, cq_head, 
          cq_tail, in_flight, retrieved, next_token, produced, pc, stack

(* define statement *)
SPSCEmpty == spsc_head = spsc_tail
SPSCFull  == (spsc_tail + 1) % SPSC_SIZE = spsc_head
SQFull    == (sq_tail + 1) % URING_SIZE = sq_head
CQFull    == (cq_tail + 1) % URING_SIZE = cq_head
CQEmpty   == cq_head = cq_tail

VARIABLES val_, ret, val

vars == << spsc, spsc_head, spsc_tail, sq, sq_head, sq_tail, cq, cq_head, 
           cq_tail, in_flight, retrieved, next_token, produced, pc, stack, 
           val_, ret, val >>

ProcSet == {"producer"}

Init == (* Global variables *)
        /\ spsc = [i \in 0..SPSC_SIZE-1 |-> [type |-> "read", token |-> 0, key |-> 0]]
        /\ spsc_head = 0
        /\ spsc_tail = 0
        /\ sq = [i \in 0..URING_SIZE-1 |-> 0]
        /\ sq_head = 0
        /\ sq_tail = 0
        /\ cq = [i \in 0..URING_SIZE-1 |-> 0]
        /\ cq_head = 0
        /\ cq_tail = 0
        /\ in_flight = {}
        /\ retrieved = {}
        /\ next_token = 1
        /\ produced = 0
        (* Procedure SPSCEnqueue *)
        /\ val_ = [ self \in ProcSet |-> defaultInitValue]
        (* Procedure SPSCDequeue *)
        /\ ret = [ self \in ProcSet |-> [type |-> "read", token |-> 0, key |-> 0]]
        (* Procedure SQEnqueue *)
        /\ val = [ self \in ProcSet |-> defaultInitValue]
        /\ stack = [self \in ProcSet |-> << >>]
        /\ pc = [self \in ProcSet |-> "Produce"]

AwaitSpace_(self) == /\ pc[self] = "AwaitSpace_"
                     /\ ~SPSCFull
                     /\ pc' = [pc EXCEPT ![self] = "SPSCEnq"]
                     /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq, sq_head, 
                                     sq_tail, cq, cq_head, cq_tail, in_flight, 
                                     retrieved, next_token, produced, stack, 
                                     val_, ret, val >>

SPSCEnq(self) == /\ pc[self] = "SPSCEnq"
                 /\ spsc' = [spsc EXCEPT ![spsc_tail % SPSC_SIZE] = val_[self]]
                 /\ spsc_tail' = spsc_tail + 1
                 /\ pc' = [pc EXCEPT ![self] = "Error"]
                 /\ UNCHANGED << spsc_head, sq, sq_head, sq_tail, cq, cq_head, 
                                 cq_tail, in_flight, retrieved, next_token, 
                                 produced, stack, val_, ret, val >>

SPSCEnqueue(self) == AwaitSpace_(self) \/ SPSCEnq(self)

AwaitResult(self) == /\ pc[self] = "AwaitResult"
                     /\ ~SPSCEmpty
                     /\ pc' = [pc EXCEPT ![self] = "SPSCDeq"]
                     /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq, sq_head, 
                                     sq_tail, cq, cq_head, cq_tail, in_flight, 
                                     retrieved, next_token, produced, stack, 
                                     val_, ret, val >>

SPSCDeq(self) == /\ pc[self] = "SPSCDeq"
                 /\ ret' = [ret EXCEPT ![self] = spsc[spsc_head % SPSC_SIZE]]
                 /\ spsc_head' = spsc_head + 1
                 /\ pc' = [pc EXCEPT ![self] = "Error"]
                 /\ UNCHANGED << spsc, spsc_tail, sq, sq_head, sq_tail, cq, 
                                 cq_head, cq_tail, in_flight, retrieved, 
                                 next_token, produced, stack, val_, val >>

SPSCDequeue(self) == AwaitResult(self) \/ SPSCDeq(self)

AwaitSpace_S(self) == /\ pc[self] = "AwaitSpace_S"
                      /\ ~SQFull
                      /\ pc' = [pc EXCEPT ![self] = "SQEnq"]
                      /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq, sq_head, 
                                      sq_tail, cq, cq_head, cq_tail, in_flight, 
                                      retrieved, next_token, produced, stack, 
                                      val_, ret, val >>

SQEnq(self) == /\ pc[self] = "SQEnq"
               /\ sq' = [sq EXCEPT ![sq_tail % URING_SIZE] = val[self]]
               /\ sq_tail' = sq_tail + 1
               /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
               /\ val' = [val EXCEPT ![self] = Head(stack[self]).val]
               /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
               /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq_head, cq, 
                               cq_head, cq_tail, in_flight, retrieved, 
                               next_token, produced, val_, ret >>

SQEnqueue(self) == AwaitSpace_S(self) \/ SQEnq(self)

Produce == /\ pc["producer"] = "Produce"
           /\ IF produced < NUM_OPS
                 THEN /\ pc' = [pc EXCEPT !["producer"] = "AwaitSpace"]
                 ELSE /\ pc' = [pc EXCEPT !["producer"] = "Done"]
           /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq, sq_head, sq_tail, 
                           cq, cq_head, cq_tail, in_flight, retrieved, 
                           next_token, produced, stack, val_, ret, val >>

AwaitSpace == /\ pc["producer"] = "AwaitSpace"
              /\ ~SPSCFull
              /\ pc' = [pc EXCEPT !["producer"] = "Push"]
              /\ UNCHANGED << spsc, spsc_head, spsc_tail, sq, sq_head, sq_tail, 
                              cq, cq_head, cq_tail, in_flight, retrieved, 
                              next_token, produced, stack, val_, ret, val >>

Push == /\ pc["producer"] = "Push"
        /\ spsc' = [spsc EXCEPT ![spsc_tail % SPSC_SIZE] = [type |-> "read", token |-> next_token, key |-> 0]]
        /\ spsc_tail' = spsc_tail + 1
        /\ next_token' = next_token + 1
        /\ produced' = produced + 1
        /\ pc' = [pc EXCEPT !["producer"] = "Produce"]
        /\ UNCHANGED << spsc_head, sq, sq_head, sq_tail, cq, cq_head, cq_tail, 
                        in_flight, retrieved, stack, val_, ret, val >>

Producer == Produce \/ AwaitSpace \/ Push

(* Allow infinite stuttering to prevent deadlock on termination. *)
Terminating == /\ \A self \in ProcSet: pc[self] = "Done"
               /\ UNCHANGED vars

Next == Producer
           \/ (\E self \in ProcSet:  \/ SPSCEnqueue(self) \/ SPSCDequeue(self)
                                     \/ SQEnqueue(self))
           \/ Terminating

Spec == Init /\ [][Next]_vars

Termination == <>(\A self \in ProcSet: pc[self] = "Done")

\* END TRANSLATION 
=============================================================
