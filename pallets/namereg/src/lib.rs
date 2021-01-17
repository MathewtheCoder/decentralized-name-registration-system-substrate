#![cfg_attr(not(feature = "std"), no_std)]

use sp_std::prelude::*;
use frame_support::{
	// codec::{Decode, Encode},
	decl_module, decl_event, decl_storage, ensure, decl_error, debug,
	traits::{Currency, EnsureOrigin, ReservableCurrency, OnUnbalanced, Get},
};
use frame_system::{self as system, ensure_signed};
use sp_runtime::traits::Saturating;
use sp_runtime::SaturatedConversion;

type BalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Trait>::AccountId>>::Balance;
type NegativeImbalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Trait>::AccountId>>::NegativeImbalance;
/// Struct to store the details of each Name
// #[derive(Encode, Decode, Clone, Default, RuntimeDebug)]
// #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
// pub struct NameStruct<BlockNumber> {
//     pub registered_name: Vec<u8>,
//     pub expiry_block_number: BlockNumber,
// }
pub trait Config: frame_system::Trait {
    // The runtime must supply this pallet with an Event type that satisfies the pallet's requirements.
    type Event: From<Event<Self>> + Into<<Self as frame_system::Trait>::Event>;

    // The currency type that will be used to place deposits on nicks.
    // It must implement ReservableCurrency.
    // https://substrate.dev/rustdocs/v2.0.0/frame_support/traits/trait.ReservableCurrency.html
    type Currency: ReservableCurrency<Self::AccountId>;

    // The amount required to reserve a name.
    type ReservationFee: Get<BalanceOf<Self>>;

    // A callback that will be invoked when a deposit is forfeited.
    type Slashed: OnUnbalanced<NegativeImbalanceOf<Self>>;

    // Origins are used to identify network participants and control access.
    // This is used to identify the pallet's admin.
    // https://substrate.dev/docs/en/knowledgebase/runtime/origin
    type ForceOrigin: EnsureOrigin<Self::Origin>;

    // This parameter is used to configure a nick's minimum length.
    type MinLength: Get<usize>;

    // This parameter is used to configure a nick's maximum length.
    // https://substrate.dev/docs/en/knowledgebase/runtime/storage#create-bounds
    type MaxLength: Get<usize>;
}

decl_storage! {
	trait Store for Module<T: Config> as NameRegistry {
		/// The lookup table for names.
		NameOf: map hasher(twox_64_concat) T::AccountId => Option<(Vec<u8>, T::BlockNumber, BalanceOf<T>)>;
		// Reverse lookup of name to (accountid, )
		RLookup: map hasher(twox_64_concat) Vec<u8> => Option<(T::AccountId, T::BlockNumber)>;
		// TODO: On accessing the data from ui its comes out as hex value only. Check later.
		// NameOf: map hasher(blake2_128_concat) T::AccountId => NameStruct<T::BlockNumber>;
	}
}

decl_event!(
	pub enum Event<T>
	where
		AccountId = <T as frame_system::Trait>::AccountId,
		Balance = BalanceOf<T>,
		BlockNumber = <T as system::Trait>::BlockNumber,
	{
		/// A name was registered. \[who, BlockNumber\]
		NameRegistered(AccountId, BlockNumber),
		/// A name was renewed. \[who, BlockNumber\]
		NameRenewed(AccountId, BlockNumber),
		/// A name was cleared, and the given balance returned. \[who, deposit\]
		NameCleared(AccountId, Balance),
	}
);

decl_error! {
	/// Error for the nicks module.
	pub enum Error for Module<T: Config> {
		// Name is owned by the txn account and is within expiry limit.
		NameAlreadyOwned,
		// Name is owned by some other account and its within the expiry limit
		NameOwnedByAnother,
		/// A name is too short.
		TooShort,
		/// A name is too long.
		TooLong,
		// Name is not registered
		NameNotRegistered,
		// No remaining blocks to refund.
		NoRemainingBlocksToRefund,
	}
}

decl_module! {
	/// NameRegistry module declaration.
	pub struct Module<T: Config> for enum Call where origin: T::Origin {
		type Error = Error<T>;

		fn deposit_event() = default;

		/// Reservation fee.
		const ReservationFee: BalanceOf<T> = T::ReservationFee::get();

		/// The minimum length a name may be.
		const MinLength: u32 = T::MinLength::get() as u32;

		/// The maximum length a name may be.
		const MaxLength: u32 = T::MaxLength::get() as u32;

		/// Register an account's name for numBlocks. The name should be a UTF-8-encoded string by convention, though
		/// we don't check it.
		///
		/// The name may not be more than `T::MaxLength` bytes, nor less than `T::MinLength` bytes.
		///
		/// If the account doesn't already have a name, then a fee of name-fee * numBlocks is reserved
		/// in the account.
		///
		/// The dispatch origin for this call must be _Signed_.
		///
		/// # <weight>
		/// - O(1).
		/// - At most one balance operation.
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 50_000_000]
		fn register(origin, name: Vec<u8>, num_blocks: u32) {
			let sender = ensure_signed(origin)?;

			ensure!(name.len() >= T::MinLength::get(), Error::<T>::TooShort);
			ensure!(name.len() <= T::MaxLength::get(), Error::<T>::TooLong);
			let current_block_number = <system::Module<T>>::block_number();
			debug::info!("Current BlockNumber: {:?}", current_block_number.clone());
			if let Some((account_id, name_expiry_block)) = <RLookup<T>>::get(&name) {
				ensure!(sender.clone() != account_id, Error::<T>::NameAlreadyOwned);
				// reverse look the account id and expiry block no and check if name is owned by someother account
				ensure!(name_expiry_block <= current_block_number, Error::<T>::NameOwnedByAnother);
			};
			// Multiply the number of blocks with the reservation fee for one block.
			let deposit = T::ReservationFee::get() * num_blocks.into();
			T::Currency::reserve(&sender, deposit.clone())?;
			let expiry_block_number = current_block_number.clone() + num_blocks.into();
			// Store the account id to (name, expiryBlock, totalDeposit) mapping
			<NameOf<T>>::insert(&sender, (name.clone(), expiry_block_number.clone(), deposit));
			// Store the name to (accountID, expiryBlock) mapping
			<RLookup<T>>::insert(name, (&sender, expiry_block_number.clone()));
			Self::deposit_event(RawEvent::NameRegistered(sender.clone(), current_block_number.clone()));
		}
		/// renew or extend the registration by locking more deposit 
		/// proportinal to name-fee per block. 
		#[weight = 70_000_000]
		fn renew(origin, name: Vec<u8>, num_blocks: u32) {
			let sender = ensure_signed(origin)?;

			ensure!(name.len() >= T::MinLength::get(), Error::<T>::TooShort);
			ensure!(name.len() <= T::MaxLength::get(), Error::<T>::TooLong);
			let current_block_number = <system::Module<T>>::block_number();
			debug::info!("Current BlockNumber: {:?}", current_block_number.clone());
			let name_expiry_block = if let Some((account_id, name_expiry_block)) = <RLookup<T>>::get(&name) {
				// Check if name owned by source account
				ensure!(sender.clone() == account_id, Error::<T>::NameOwnedByAnother);
				name_expiry_block
			} else {
				// Temp hack to throw error
				ensure!(false, Error::<T>::NameNotRegistered);
				// To make the types same
				current_block_number
			};
			// Multiply the number of blocks with the reservation fee for one block.
			let deposit = T::ReservationFee::get() * num_blocks.into();
			T::Currency::reserve(&sender, deposit.clone())?;
			let mut expiry_block_number = current_block_number.clone() + num_blocks.into();
			// Check if name is not expired and if so add numBlocks to existing expiry number.
			if name_expiry_block > current_block_number {
				expiry_block_number = name_expiry_block + num_blocks.into()
			}
			// Store the account id to (name, expiryBlock, totalDeposit) mapping
			<NameOf<T>>::insert(&sender, (name.clone(), expiry_block_number.clone(), deposit));
			// Store the name to (accountID, expiryBlock) mapping
			<RLookup<T>>::insert(name, (&sender, expiry_block_number.clone()));
			Self::deposit_event(RawEvent::NameRenewed(sender.clone(), current_block_number.clone()));
		}

		/// Clear an account's name and return the remaining deposit.
		/// Fails if the account was not registered.
		///
		/// The dispatch origin for this call must be _Signed_.
		///
		/// # <weight>
		/// - O(1).
		/// - One balance operation.
		/// - One storage read/write.
		/// - One event.
		/// # </weight>
		#[weight = 70_000_000]
		fn clear(origin, name: Vec<u8>) {
			let sender = ensure_signed(origin)?;
			
			let name_of_tuple = <NameOf<T>>::take(&sender).ok_or(Error::<T>::NameNotRegistered);
			// Get the owner account id from name
			let name_owner_account_id = <RLookup<T>>::take(name.clone()).ok_or(Error::<T>::NameNotRegistered)?.0;
			match name_of_tuple {
				Ok((name_temp, expiry_block_number, deposit)) => {
					// Is the name entered in function belong to source account
					ensure!(name.clone() == name_temp, Error::<T>::NameOwnedByAnother);
					// Does Name belongs to the source account
					ensure!(sender.clone() == name_owner_account_id, Error::<T>::NameOwnedByAnother);
					let current_block_number = <system::Module<T>>::block_number();
					debug::info!("Current BlockNumber: {:?}", current_block_number.clone());
					let remaining_blocks = expiry_block_number - current_block_number;
					// Is there any remaining blocks to refund
					if remaining_blocks > 0.into() {
						let refund_amount = T::ReservationFee::get().saturated_into::<u64>() * remaining_blocks.saturated_into::<u64>();
						let _ = T::Currency::unreserve(&sender, refund_amount.saturated_into());
						Self::deposit_event(RawEvent::NameCleared(sender, refund_amount.saturated_into()));
					} else {
						// Throw error saying no deposit can be refunded
						ensure!(false, Error::<T>::NoRemainingBlocksToRefund);
					}
				},
				Err(_) => debug::info!("It doesn't matter what they are"),
			};
		}
	}
}