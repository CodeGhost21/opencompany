use super::*;

fn spawn(title: &str) -> Delegation {
    Delegation::SpawnTask {
        title: title.to_string(),
        note: None,
        assignee: None,
    }
}

fn hand_off() -> Delegation {
    Delegation::DelegateToDesk {
        desk: "design".to_string(),
        instruction: "look".to_string(),
    }
}

#[tokio::test]
async fn a_seat_claim_stages_cards_and_refuses_everything_else_on_the_board() {
    let queue = DelegationQueue::default();
    let claim = queue.claim_seat("ep:writer", false);
    let (card, other) = claim
        .scoped(async {
            (
                queue.push_within_cap(spawn("Draft"), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND),
                queue.push_within_cap(hand_off(), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND),
            )
        })
        .await;
    assert_eq!(card, Staged::Queued);
    assert_eq!(other, Staged::NoDrain(NoDrainReason::Seat));
    assert_eq!(
        queue.push_within_cap(
            Delegation::AssignTask {
                task_id: "t".to_string(),
                assignee: "x".to_string(),
                note: None,
            },
            MAX_DELEGATIONS_PER_TURN,
            NO_DEPTH_BOUND
        ),
        Staged::NoDrain(NoDrainReason::Unwired),
        "outside the seat's scope nothing has claimed the pooled bucket"
    );
    assert_eq!(claim.drain(MAX_DELEGATIONS_PER_TURN), vec![spawn("Draft")]);
}

#[tokio::test]
async fn a_seats_bucket_is_invisible_to_the_pooled_drain_and_to_another_seat() {
    let queue = DelegationQueue::default();
    let pooled = queue.claim();
    let writer = queue.claim_seat("ep:writer", false);
    let analyst = queue.claim_seat("ep:analyst", false);
    writer
        .scoped(async {
            let _ = queue.push_within_cap(spawn("Mine"), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND);
        })
        .await;
    assert!(queue.drain(MAX_DELEGATIONS_PER_TURN).is_empty());
    assert!(analyst.drain(MAX_DELEGATIONS_PER_TURN).is_empty());
    assert_eq!(writer.drain(MAX_DELEGATIONS_PER_TURN), vec![spawn("Mine")]);
    drop(pooled);
}

#[tokio::test]
async fn a_seat_answering_a_question_refuses_cards_as_a_pooled_question_turn_does() {
    let queue = DelegationQueue::default();
    let claim = queue.claim_seat("ep:writer", true);
    let staged = claim
        .scoped(async {
            queue.push_within_cap(spawn("Draft"), MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND)
        })
        .await;
    assert_eq!(staged, Staged::NoDrain(NoDrainReason::Triage));
    assert!(claim.drain(MAX_DELEGATIONS_PER_TURN).is_empty());
}

#[test]
fn the_seat_refusal_points_at_asking_a_teammate() {
    let text = no_drain(SPAWN_TASK_TOOL, "the card was NOT opened", NoDrainReason::Seat);
    assert!(text.contains("desk_ask"), "{text}");
    assert!(text.starts_with("Refused"), "{text}");
}
