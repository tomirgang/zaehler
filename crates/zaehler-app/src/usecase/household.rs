//! Haushalte anlegen, ändern und löschen.

use crate::{
    AppError, FieldErrors, Role, Tenant,
    ids::{HouseholdId, UserId},
    model::{Household, HouseholdData},
    usecase::{use_case, validate},
};

pub const MAX_HOUSEHOLD_NAME: usize = 80;

fn validate_data(data: &HouseholdData) -> Result<(), AppError> {
    let mut errors = FieldErrors::default();
    validate::name(
        &mut errors,
        "name",
        &data.name,
        MAX_HOUSEHOLD_NAME,
        "household.name_invalid",
    );
    errors.into_result()
}

fn trimmed(data: &HouseholdData) -> HouseholdData {
    HouseholdData {
        name: data.name.trim().to_owned(),
        timezone: data.timezone,
    }
}

use_case!(
    /// Weiteren Haushalt anlegen; der Nutzer wird Owner.
    CreateHousehold,
    GetHousehold,
    UpdateHousehold,
    /// Haushalt mit allen Daten löschen (nach Bestätigung durch den Namen).
    DeleteHousehold,
);

impl CreateHousehold {
    // @impl SPEC_TEN_HOUSEHOLDS
    pub async fn execute(
        &self,
        user: UserId,
        data: HouseholdData,
    ) -> Result<HouseholdId, AppError> {
        validate_data(&data)?;
        let id = HouseholdId(self.p.ids.new_uuid());
        let mut uow = self.p.uow.begin().await?;
        uow.households()
            .create(id, user, &trimmed(&data), self.p.clock.now())
            .await?;
        uow.commit().await?;
        Ok(id)
    }
}

impl GetHousehold {
    pub async fn execute(&self, t: &Tenant) -> Result<Household, AppError> {
        let mut uow = self.p.uow.begin().await?;
        Ok(uow.households().get(t).await?)
    }
}

impl UpdateHousehold {
    // @impl SPEC_TEN_HOUSEHOLDS
    pub async fn execute(&self, t: &Tenant, data: HouseholdData) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        validate_data(&data)?;
        let mut uow = self.p.uow.begin().await?;
        uow.households().update(t, &trimmed(&data)).await?;
        uow.commit().await?;
        Ok(())
    }
}

impl DeleteHousehold {
    // @impl SPEC_PRIV_HOUSEHOLD_DELETE
    pub async fn execute(&self, t: &Tenant, confirm_name: &str) -> Result<(), AppError> {
        t.require(Role::Owner)?;
        let mut uow = self.p.uow.begin().await?;
        let household = uow.households().get(t).await?;
        if confirm_name.trim() != household.data.name {
            return Err(AppError::field(
                "confirm",
                "household.delete_confirm_mismatch",
            ));
        }
        // Objekte zuerst: schlägt das Löschen fehl, bleibt der Haushalt
        // bestehen und das Löschen kann wiederholt werden.
        self.p
            .blobs
            .delete_household(t.household_id())
            .await
            .map_err(AppError::from_storage)?;
        uow.households().delete(t).await?;
        uow.commit().await?;
        tracing::info!(household = %t.household_id(), "household deleted");
        Ok(())
    }
}
