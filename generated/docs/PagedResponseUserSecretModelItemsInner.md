# PagedResponseUserSecretModelItemsInner

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**bucket_name** | Option<**String**> |  | [optional]
**client** | [**models::ClientType**](ClientType.md) |  | 
**concurrency** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**created_at** | **String** |  | 
**deleted_at** | Option<**String**> |  | [optional]
**id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 
**image_collections** | [**Vec<models::ImageCollection>**](ImageCollection.md) |  | 
**name** | **String** |  | 
**secret_value** | Option<[**serde_json::Value**](.md)> |  | 
**storage_retention_id** | Option<[**uuid::Uuid**](uuid::Uuid.md)> |  | [optional]
**updated_at** | **String** |  | 
**user_id** | [**uuid::Uuid**](uuid::Uuid.md) |  | 

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


