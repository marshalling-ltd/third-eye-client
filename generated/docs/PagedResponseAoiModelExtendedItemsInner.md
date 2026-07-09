# PagedResponseAoiModelExtendedItemsInner

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**area** | Option<[**serde_json::Value**](.md)> |  | 
**concurrency** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**created_at** | **String** |  | 
**deleted_at** | Option<**String**> |  | [optional]
**external_id** | Option<**String**> |  | [optional]
**id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**image_collection** | [**models::ImageCollection**](ImageCollection.md) |  | 
**last_image_download_at** | **String** |  | 
**logo** | Option<**String**> |  | [optional]
**name** | **String** |  | 
**name_url** | **String** |  | 
**options** | Option<[**serde_json::Value**](.md)> |  | 
**schedule** | Option<**String**> |  | [optional]
**show_all_images** | **bool** |  | 
**source_secret_id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**source_secret_name** | Option<**String**> |  | [optional]
**source_secret_type** | Option<[**models::ClientType**](ClientType.md)> |  | [optional]
**storage_secret_id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**storage_secret_name** | Option<**String**> |  | [optional]
**storage_secret_type** | Option<[**models::ClientType**](ClientType.md)> |  | [optional]
**tags** | [**Vec<models::TagEntityModel>**](TagEntityModel.md) |  | 
**updated_at** | **String** |  | 
**user_id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


